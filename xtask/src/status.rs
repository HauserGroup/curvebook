//! `cargo xtask status`: verify unique first-column keys in `docs/STATUS.md` and `docs/ROADMAP.md`,
//! live metric parity, and human-path test-evidence gates across `docs/evidence/user-interface.md`
//! and `docs/gui-workflows.md` §6.2.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::rust_lex::{blank_comments_and_literals_both, count_newlines, find_fn_body_braces, find_matching_delim, is_ident_byte};
use crate::scan_util::{format_error_list, has_ext_ci, is_real_dir, read_dir_paths_sorted, rel_slash_lossy, walk_file_paths};

const STATUS_REL: &str = "docs/STATUS.md";
const ROADMAP_REL: &str = "docs/ROADMAP.md";
const UI_EVIDENCE_REL: &str = "docs/evidence/user-interface.md";
const GUI_WORKFLOWS_REL: &str = "docs/gui-workflows.md";
const AGENT_HARNESS_TAG: &str = "*(agent harness)*";
const HUMAN_DRIVER_TOKENS: &[&str] = &["humanpathdriver", "eguihumanpathdriver", "egui_kittest"];
const KNOWN_BACKENDS: &[&str] = &["legacy", "egui"];
const TOTAL_WORKFLOWS: usize = 65;
const TOTAL_WORKFLOWS_U8: u8 = 65;
const AGENT_HARNESS_MARKERS: &str = "--control --event-script --gui-only dispatch_gui_action dispatch_control_line run_event_script_jsonl ui.invoke_menu ui.answer_dialog ui.set_field ui.click ui.press_key ui.inspect ui.screenshot ui.pointer_gesture ui.type_text ui.switch_tab ui_inspect ui_screenshot ui_pointer_gesture ui_type_text ui_press_key ui_invoke_menu ui_switch_tab ui_answer_dialog gui_only_workflows gui_w22b_qol_and_r9 gui_only_surface mcp_gui_only_parity gui_only_wiring jsonl_script_playback control_loopback control_auth_and_panic";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateKey {
    pub path: String,
    pub line: usize,
    pub key: String,
}
impl std::fmt::Display for DuplicateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: duplicate column-1 table key `{}`", self.path, self.line, self.key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceViolation {
    pub path: String,
    pub line: usize,
    pub reason: String,
}
impl std::fmt::Display for EvidenceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.path, self.line, self.reason)
    }
}
fn push_ev(out: &mut Vec<EvidenceViolation>, path: &str, line: usize, reason: impl Into<String>) {
    out.push(EvidenceViolation { path: path.to_owned(), line, reason: reason.into() });
}

#[derive(Debug, Default, Clone)]
pub struct WorkspaceTests {
    pub unqualified: BTreeSet<String>,
    pub qualified: BTreeSet<String>,
}
impl WorkspaceTests {
    #[must_use]
    pub fn contains_test(&self, token: &str) -> bool {
        if token.contains("::") { self.qualified.contains(token) } else { self.unqualified.contains(token) }
    }
}

pub fn run(root: &Path) -> Result<(), String> {
    let (mut total_tables, mut duplicates) = (0usize, Vec::new());
    let status_text = fs::read_to_string(root.join(STATUS_REL)).map_err(|e| format!("{STATUS_REL}: {e}"))?;
    let roadmap_text = fs::read_to_string(root.join(ROADMAP_REL)).map_err(|e| format!("{ROADMAP_REL}: {e}"))?;
    for (rel, text) in [(STATUS_REL, &status_text), (ROADMAP_REL, &roadmap_text)] {
        let (tables, mut dups) = check_markdown_tables(rel, text);
        if tables == 0 {
            return Err(format!("{rel}: expected at least one Markdown table"));
        }
        total_tables += tables;
        duplicates.append(&mut dups);
    }
    let mut violations = check_live_status_metrics(root, &status_text);
    let workspace_tests = collect_workspace_tests(root)?;
    let (complete_wfs, mut wf_violations) = scan_human_wf_completion(root)?;

    let ui_text = fs::read_to_string(root.join(UI_EVIDENCE_REL)).map_err(|e| format!("{UI_EVIDENCE_REL}: {e}"))?;
    violations.extend(check_ui_parity_evidence(UI_EVIDENCE_REL, &ui_text, Some(&workspace_tests)));
    violations.extend(check_ui_parity_five_stage_claims(UI_EVIDENCE_REL, &ui_text, &complete_wfs));
    let (telemetry, tel_v) = check_backend_workflow_telemetry(UI_EVIDENCE_REL, &ui_text, &workspace_tests);
    violations.extend(tel_v);

    let wf_text = fs::read_to_string(root.join(GUI_WORKFLOWS_REL)).map_err(|e| format!("{GUI_WORKFLOWS_REL}: {e}"))?;
    violations.extend(check_gui_workflows_human_path_with_completion(GUI_WORKFLOWS_REL, &wf_text, Some(&workspace_tests), Some(&complete_wfs)));
    violations.append(&mut wf_violations);

    if duplicates.is_empty() && violations.is_empty() {
        println!(
            "status: verified {total_tables} Markdown table(s) across {STATUS_REL} and {ROADMAP_REL}, live metrics, [x] evidence in {UI_EVIDENCE_REL}, and §6.2 Human path column in {GUI_WORKFLOWS_REL} ({}/65 egui; backed human tests: {}/65 egui; five-stage complete: {}/65)",
            telemetry.egui_claimed,
            telemetry.egui_backed,
            complete_wfs.len()
        );
        return Ok(());
    }
    let mut errors = Vec::new();
    if !duplicates.is_empty() {
        errors.push(format_error_list("duplicate table key(s) found", &duplicates));
    }
    if !violations.is_empty() {
        errors.push(format_error_list("missing/invalid [x] evidence violation(s)", &violations));
    }
    Err(errors.join("\n"))
}

pub fn collect_workspace_tests(root: &Path) -> Result<WorkspaceTests, String> {
    let (mut tests, mut paths) = (WorkspaceTests::default(), Vec::new());
    for dir in ["crates", "apps"] {
        let base = root.join(dir);
        if is_real_dir(&base) {
            walk_file_paths(
                &base,
                &|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == "target" || n.starts_with('.')),
                &|p| has_ext_ci(p, &["rs"]),
                &mut paths,
            )?;
        }
    }
    for path in paths {
        let src = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let parent = matches!(stem, "mod" | "lib" | "main").then(|| path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str())).flatten();
        scan_tests_in_rust_source(stem, parent, &src, &mut tests);
    }
    Ok(tests)
}

fn scan_tests_in_rust_source(stem: &str, parent_mod: Option<&str>, src: &str, out: &mut WorkspaceTests) {
    let (code_only, with_strings) = blank_comments_and_literals_both(src, false);
    for f in scan_rust_fn_spans(&code_only, &with_strings).into_iter().filter(|f| f.is_test && !f.is_ignored) {
        out.unqualified.insert(f.name.to_owned());
        if !stem.is_empty() {
            out.qualified.insert(format!("{stem}::{}", f.name));
        }
        if let Some(p) = parent_mod.filter(|p| !p.is_empty()) {
            out.qualified.insert(format!("{p}::{}", f.name));
        }
    }
}

struct ScannedRustFn<'a> {
    is_test: bool,
    is_ignored: bool,
    name: &'a str,
    line: usize,
    header_and_body_code: &'a str,
    body_with_strings: &'a str,
    body_code_only: &'a str,
}

fn scan_rust_fn_spans<'a>(code_only: &'a str, with_strings: &'a str) -> Vec<ScannedRustFn<'a>> {
    let (bytes, mut out, mut i, mut line_no, mut p_test, mut p_ign) = (code_only.as_bytes(), Vec::new(), 0usize, 1usize, false, false);
    while i < bytes.len() {
        if bytes.get(i..i + 2) == Some(b"#[")
            && let Some(close) = find_matching_delim(bytes, i + 1, b'[', b']')
        {
            let attr = code_only.get(i..=close).unwrap_or("");
            p_test |= attr.starts_with("#[test]") || attr.contains("::test]") || attr.contains("::test(");
            p_ign |= attr.starts_with("#[ignore");
            (line_no, i) = (line_no + count_newlines(&bytes[i..=close]), close + 1);
            continue;
        }
        if is_word_at(code_only, i, "fn") && can_start_item_before(code_only.get(..i).unwrap_or("")) {
            let mut p = i + 2;
            while bytes.get(p).is_some_and(u8::is_ascii_whitespace) {
                p += 1;
            }
            let ns = p;
            while bytes.get(p).is_some_and(|b| is_ident_byte(*b)) {
                p += 1;
            }
            let name = code_only.get(ns..p).unwrap_or("");
            if !name.is_empty()
                && let Some((open, close)) = find_fn_body_braces(bytes, p)
            {
                out.push(ScannedRustFn {
                    is_test: p_test,
                    is_ignored: p_ign,
                    name,
                    line: line_no,
                    header_and_body_code: code_only.get(i..=close).unwrap_or(""),
                    body_with_strings: with_strings.get(open..=close).unwrap_or(""),
                    body_code_only: code_only.get(open..=close).unwrap_or(""),
                });
                (line_no, i, p_test, p_ign) = (line_no + count_newlines(&bytes[i..=close]), close + 1, false, false);
                continue;
            }
        }
        if matches!(bytes[i], b';' | b'{' | b'}') {
            (p_test, p_ign) = (false, false);
        }
        (line_no, i) = (line_no + usize::from(bytes[i] == b'\n'), i + 1);
    }
    out
}

fn can_start_item_before(prefix: &str) -> bool {
    let t = prefix.trim_end();
    t.is_empty()
        || t.bytes().last().is_some_and(|b| matches!(b, b';' | b'{' | b'}' | b']' | b')' | b'"'))
        || matches!(t.rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or(""), "pub" | "async" | "const" | "unsafe")
}

#[must_use]
pub fn check_ui_parity_evidence(path: &str, md: &str, known_tests: Option<&WorkspaceTests>) -> Vec<EvidenceViolation> {
    let (mut violations, mut fence) = (Vec::new(), None);
    for (idx, line) in md.lines().enumerate() {
        let t = line.trim();
        if !update_fence(t, &mut fence)
            && fence.is_none()
            && is_checked_markdown_item(t)
            && !line.to_ascii_lowercase().contains(AGENT_HARNESS_TAG)
            && let Some(v) = validate_human_evidence(path, idx + 1, line, known_tests)
        {
            violations.push(v);
        }
    }
    violations
}

#[must_use]
pub fn check_gui_workflows_human_path_with_completion(
    path: &str,
    md: &str,
    known_tests: Option<&WorkspaceTests>,
    complete_wfs: Option<&BTreeSet<u8>>,
) -> Vec<EvidenceViolation> {
    let (mut violations, mut fence, mut in_sec, mut human_col, mut saw_sep, mut verified) = (Vec::new(), None, false, None::<usize>, false, false);
    for (idx, line) in md.lines().enumerate() {
        let trimmed = line.trim();
        if update_fence(trimmed, &mut fence) || fence.is_some() {
            continue;
        }
        if trimmed.to_ascii_lowercase().starts_with("### 6.2") {
            (in_sec, human_col, saw_sep) = (true, None, false);
            continue;
        }
        if in_sec && (trimmed.starts_with("## ") || trimmed.starts_with("### ")) {
            (in_sec, human_col, saw_sep) = (false, None, false);
            continue;
        }
        if !in_sec || !is_table_row(trimmed) {
            if human_col.is_some() && !trimmed.is_empty() {
                (human_col, saw_sep) = (None, false);
            }
            continue;
        }
        let cells = split_table_cells(trimmed);
        let Some(col) = human_col else {
            human_col = cells.iter().position(|c| c.trim().trim_matches(['*', '_', '`', ' ']).eq_ignore_ascii_case("human path"));
            (saw_sep, verified) = (false, verified | human_col.is_some());
            continue;
        };
        if !saw_sep && is_table_separator(trimmed) {
            saw_sep = true;
            continue;
        }
        if let Some(cell) = cells.get(col) {
            if human_path_requires_evidence(cell)
                && let Some(v) = validate_human_evidence(path, idx + 1, cell, known_tests)
            {
                violations.push(v);
            }
            if let Some(complete_set) = complete_wfs {
                validate_section62_completion_counts(path, idx + 1, cells.first().copied().unwrap_or(""), cell, complete_set, &mut violations);
            }
        }
    }
    if !verified {
        push_ev(&mut violations, path, 1, "missing §6.2 table with 'Human path' column");
    }
    violations
}

fn update_fence(trimmed: &str, fence: &mut Option<char>) -> bool {
    let Some(c) = trimmed.starts_with("```").then_some('`').or_else(|| trimmed.starts_with("~~~").then_some('~')) else { return false };
    match *fence {
        Some(a) if a == c => *fence = None,
        None => *fence = Some(c),
        _ => {}
    }
    true
}

fn is_checked_markdown_item(trimmed: &str) -> bool {
    let is_box = |s: &str| matches!(s, "[x]" | "[X]") || s.starts_with("[x] ") || s.starts_with("[X] ");
    if is_table_row(trimmed) {
        return split_table_cells(trimmed).into_iter().any(|c| is_box(c.trim()));
    }
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|p| trimmed.strip_prefix(p).map(str::trim_start))
        .or_else(|| {
            let (p, r) = trimmed.split_once(['.', ')'])?;
            (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).then(|| r.strip_prefix(' ').map(str::trim_start)).flatten()
        })
        .is_some_and(is_box)
}

fn split_table_cells(trimmed: &str) -> Vec<&str> {
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    inner.strip_suffix('|').unwrap_or(inner).split('|').collect()
}

fn human_path_requires_evidence(cell: &str) -> bool {
    let lower = cell.trim().trim_matches(|c: char| matches!(c, '`' | '*' | '_') || c.is_whitespace()).to_ascii_lowercase();
    if matches!(lower.as_str(), "" | "—" | "-" | "–" | "n/a" | "no" | "none" | "todo" | "pending" | "untested" | "[ ]" | "0" | "0%") {
        return false;
    }
    let fracs = parse_workflow_fractions(&lower);
    fracs.iter().any(|&(n, d, b)| b.is_some() && (d == 0 || n > d)) || fracs.is_empty() || !fracs.iter().all(|&(n, d, _)| n == 0 && d > 0)
}

fn parse_workflow_fractions(text: &str) -> Vec<(usize, usize, Option<&str>)> {
    let (bytes, mut out) = (text.as_bytes(), Vec::new());
    for (slash, _) in bytes.iter().enumerate().filter(|&(_, &b)| b == b'/') {
        let le = bytes[..slash].iter().rposition(|b| !b.is_ascii_whitespace()).map_or(0, |p| p + 1);
        let ls = bytes[..le].iter().rposition(|b| !b.is_ascii_digit()).map_or(0, |p| p + 1);
        let rs = bytes[slash + 1..].iter().position(|b| !b.is_ascii_whitespace()).map_or(bytes.len(), |p| slash + 1 + p);
        let re = bytes[rs..].iter().position(|b| !b.is_ascii_digit()).map_or(bytes.len(), |p| rs + p);
        if ls == le || rs == re || (ls > 0 && bytes[ls - 1].is_ascii_alphanumeric()) || bytes.get(re).is_some_and(u8::is_ascii_alphanumeric) {
            continue;
        }
        if let (Some(num), Some(den)) = (text.get(ls..le).and_then(|s| s.parse().ok()), text.get(rs..re).and_then(|s| s.parse().ok())) {
            let trail = text.get(re..).unwrap_or("").trim_start().split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("");
            out.push((num, den, KNOWN_BACKENDS.iter().copied().find(|&bn| trail.eq_ignore_ascii_case(bn))));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackendWorkflowTelemetry {
    pub legacy_claimed: usize,
    pub egui_claimed: usize,
    pub legacy_backed: usize,
    pub egui_backed: usize,
}

#[must_use]
pub fn count_backed_backend_workflows(known_tests: &WorkspaceTests) -> (usize, usize) {
    let egui_ids: BTreeSet<u8> = known_tests
        .qualified
        .iter()
        .filter(|q| !q.split("::").next().unwrap_or("").starts_with("gui_wf_"))
        .filter_map(|q| {
            let rest = q.rsplit("::").next().unwrap_or(q).strip_prefix("wf")?;
            let suffix = rest.trim_start_matches(|c: char| c.is_ascii_digit());
            let idx = rest.strip_suffix(suffix)?.parse::<u8>().ok().filter(|i| (1..=TOTAL_WORKFLOWS_U8).contains(i))?;
            (matches!(suffix, "_human" | "_egui_human") || suffix.starts_with("_human_") || suffix.starts_with("_egui_human_")).then_some(idx)
        })
        .collect();
    (0, egui_ids.len())
}

#[must_use]
pub fn check_backend_workflow_telemetry(path: &str, ui_md: &str, known_tests: &WorkspaceTests) -> (BackendWorkflowTelemetry, Vec<EvidenceViolation>) {
    let (legacy_backed, egui_backed) = count_backed_backend_workflows(known_tests);
    let (mut violations, mut fence, mut legacy_claims, mut egui_claims) = (Vec::new(), None, Vec::new(), Vec::new());
    for (idx, line) in ui_md.lines().enumerate() {
        let t = line.trim();
        if update_fence(t, &mut fence)
            || fence.is_some()
            || !(t.starts_with("- [x]") || t.starts_with("- [X]") || t.starts_with("- [ ]"))
            || t.to_ascii_lowercase().contains(AGENT_HARNESS_TAG)
        {
            continue;
        }
        for (num, den, backend) in parse_workflow_fractions(&t.to_ascii_lowercase()) {
            if den == TOTAL_WORKFLOWS {
                match backend {
                    Some("legacy") => legacy_claims.push((idx + 1, num)),
                    Some("egui") => egui_claims.push((idx + 1, num)),
                    _ => {}
                }
            }
        }
    }
    let legacy_claimed =
        if legacy_claims.is_empty() { 0 } else { summarize_backend_claims(path, "legacy", &legacy_claims, legacy_backed, &mut violations) };
    let egui_claimed = summarize_backend_claims(path, "egui", &egui_claims, egui_backed, &mut violations);
    (BackendWorkflowTelemetry { legacy_claimed, egui_claimed, legacy_backed, egui_backed }, violations)
}

fn summarize_backend_claims(path: &str, backend: &str, claims: &[(usize, usize)], backed: usize, violations: &mut Vec<EvidenceViolation>) -> usize {
    let Some(&(first_line, first_num)) = claims.first() else {
        push_ev(violations, path, 1, format!("missing `N/{TOTAL_WORKFLOWS} {backend}` workflow checklist count"));
        return 0;
    };
    for &(line, num) in claims {
        if num != first_num {
            push_ev(
                violations,
                path,
                line,
                format!(
                    "conflicting `{num}/{TOTAL_WORKFLOWS} {backend}` count (line {first_line} reports `{first_num}/{TOTAL_WORKFLOWS} {backend}`)"
                ),
            );
        }
        if num > TOTAL_WORKFLOWS || num > backed {
            push_ev(
                violations,
                path,
                line,
                format!(
                    "claims `{num}/{TOTAL_WORKFLOWS} {backend}` human-path workflows, but only `{backed}/{TOTAL_WORKFLOWS}` backed non-ignored tests exist"
                ),
            );
        }
    }
    first_num
}

fn validate_human_evidence(path: &str, line: usize, text: &str, known_tests: Option<&WorkspaceTests>) -> Option<EvidenceViolation> {
    if parse_workflow_fractions(&text.to_ascii_lowercase()).iter().any(|&(n, d, b)| b.is_some() && (d == 0 || n > d)) {
        return Some(EvidenceViolation { path: path.to_owned(), line, reason: String::from("contains invalid per-backend workflow count fraction") });
    }
    let (mut seen, mut tokens) = (BTreeSet::new(), Vec::new());
    let mut push_toks = |frag: &str| {
        for raw in frag.split(|c: char| c.is_whitespace() || c == ',') {
            let c = raw.trim_matches(['`', '*', '(', ')', '[', ']', ';', '.', ':', '"', '\'']);
            if !c.is_empty() && seen.insert(c.to_owned()) {
                tokens.push(c.to_owned());
            }
        }
    };
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('`') {
        let Some((code, tail)) = after.split_once('`') else { break };
        push_toks(code);
        rest = tail;
    }
    push_toks(text);

    let (mut harness_hits, mut valid_tests, mut unknown_tests) = (Vec::new(), Vec::new(), Vec::new());
    for tok in tokens {
        let lower = tok.trim().to_ascii_lowercase();
        if HUMAN_DRIVER_TOKENS.iter().any(|&m| lower == m || lower.ends_with(&format!("::{m}"))) {
            continue;
        }
        if is_agent_harness_token(&lower) {
            harness_hits.push(tok);
        } else if is_test_identifier(&tok) {
            (if known_tests.is_none_or(|s| s.contains_test(&tok)) { &mut valid_tests } else { &mut unknown_tests }).push(tok);
        }
    }
    if !harness_hits.is_empty() {
        return Some(EvidenceViolation {
            path: path.to_owned(),
            line,
            reason: format!("cites agent-harness evidence ({}) which never proves human GUI parity", harness_hits.join(", ")),
        });
    }
    if valid_tests.is_empty() && !has_recorded_manual_run(text) {
        let reason = if unknown_tests.is_empty() {
            String::from("missing passing human-path test name or recorded computer-use/manual run")
        } else {
            format!(
                "missing passing human-path test or recorded computer-use/manual run (unmatched test identifier(s): {})",
                unknown_tests.join(", ")
            )
        };
        return Some(EvidenceViolation { path: path.to_owned(), line, reason });
    }
    None
}

fn is_agent_harness_token(lower: &str) -> bool {
    if lower.starts_with("gui_wf_")
        || lower.contains("::gui_wf_")
        || lower.contains("gui_only")
        || matches!(lower, "gui_actions" | "control_io")
        || lower.starts_with("gui_actions::")
        || lower.starts_with("control_io::")
    {
        return true;
    }
    let last = lower.rsplit("::").next().unwrap_or(lower);
    let is_wf_harness = last.strip_prefix("wf").is_some_and(|rest| {
        let suf = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        suf.len() < rest.len()
            && suf.starts_with('_')
            && !matches!(suf, "_human" | "_egui_human")
            && !suf.starts_with("_human_")
            && !suf.starts_with("_egui_human_")
    });
    is_wf_harness || AGENT_HARNESS_MARKERS.split_ascii_whitespace().any(|m| lower.contains(m))
}

fn is_test_identifier(tok: &str) -> bool {
    if tok.starts_with(['.', '-']) || tok.contains(['/', '\\', '+', '=', '>', '<', '?']) {
        return false;
    }
    if tok.contains("::") {
        let parts: Vec<&str> = tok.split("::").collect();
        return parts.len() >= 2
            && parts.iter().all(|s| s.starts_with(|c: char| c.is_ascii_lowercase()) && s.bytes().all(is_ident_byte))
            && parts.last().is_some_and(|l| l.contains('_') && l.len() >= 4);
    }
    tok.len() >= 6 && tok.contains('_') && tok.starts_with(|c: char| c.is_ascii_lowercase()) && tok.bytes().all(is_ident_byte)
}

fn has_recorded_manual_run(text: &str) -> bool {
    const COLON: &str = "manual:|manual run:|manual walkthrough:|manual-verified:|recorded manual:|manual qa:|computer-use:|computer-use run:|computer-use walkthrough:|computer-use qa:|recorded computer-use:|computer use:|computer use walkthrough:|computer use qa:";
    const PAREN: &str = "manual (|manual run (|manual walkthrough (|computer-use (|computer-use run (|computer-use walkthrough (|computer-use qa (|computer use (|computer use walkthrough (";
    let lower = text.to_ascii_lowercase();
    let not_neg = |before: &str| !matches!(before.split_whitespace().next_back().unwrap_or(""), "no" | "without");
    COLON.split('|').any(|pfx| {
        lower.split_once(pfx).is_some_and(|(b, a)| {
            let first = a
                .trim()
                .split(|c: char| c.is_whitespace() || matches!(c, ')' | ',' | ';'))
                .next()
                .unwrap_or("")
                .trim_matches(['`', '*', '_', '.', ':', ';', ',', '!', '?']);
            not_neg(b)
                && !first.is_empty()
                && !matches!(first, "none" | "no" | "todo" | "n/a" | "pending" | "tbd" | "planned" | "missing" | "needed" | "untested")
        })
    }) || PAREN.split('|').any(|m| {
        lower.split_once(m).is_some_and(|(b, a)| {
            not_neg(b)
                && a.split_once(')')
                    .is_some_and(|(in_, _)| in_.trim().contains("202") && !["todo", "pending", "no ", "none"].iter().any(|k| in_.trim().contains(k)))
        })
    })
}

pub fn scan_human_wf_completion(root: &Path) -> Result<(BTreeSet<u8>, Vec<EvidenceViolation>), String> {
    let tests_dir = root.join("crates/ui-egui/tests");
    if !is_real_dir(&tests_dir) {
        return Ok((BTreeSet::new(), Vec::new()));
    }
    let mut paths = Vec::new();
    for path in read_dir_paths_sorted(&tests_dir)? {
        let (Ok(meta), Some(name)) = (fs::symlink_metadata(&path), path.file_name().and_then(|s| s.to_str())) else { continue };
        if meta.file_type().is_symlink() || !name.starts_with("human_wf_") || name == "human_wf_common" {
            continue;
        }
        if meta.is_file() && name.ends_with(".rs") {
            paths.push(path);
        } else if meta.is_dir() {
            walk_file_paths(&path, &|_| false, &|p| has_ext_ci(p, &["rs"]), &mut paths)?;
        }
    }
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        let rel = rel_slash_lossy(root, path);
        files.push((rel.clone(), fs::read_to_string(path).map_err(|e| format!("{rel}: {e}"))?));
    }
    Ok(analyze_human_wf_sources(&files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect::<Vec<_>>()))
}

#[must_use]
pub fn analyze_human_wf_sources(sources: &[(&str, &str)]) -> (BTreeSet<u8>, Vec<EvidenceViolation>) {
    let (mut complete, mut violations) = (BTreeSet::new(), Vec::new());
    for &(path, src) in sources {
        let (code_only, with_strings) = blank_comments_and_literals_both(src, false);
        for f in scan_rust_fn_spans(&code_only, &with_strings) {
            if !f.is_test || parse_wf_fn_prefix(f.name).is_some() || f.header_and_body_code.contains("WorkflowRun") {
                check_workflow_run_lint(path, &f, &mut violations);
            }
            if f.is_test
                && !f.is_ignored
                && let Some(idx) = parse_wf_fn_prefix(f.name)
                && has_wf_start(f.body_with_strings, idx)
                && has_finish(f.body_code_only)
            {
                complete.insert(idx);
            }
        }
    }
    (complete, violations)
}

fn is_word_at(text: &str, pos: usize, word: &str) -> bool {
    text.get(pos..).is_some_and(|s| s.starts_with(word))
        && (pos == 0 || !is_ident_byte(text.as_bytes()[pos - 1]))
        && (pos + word.len() == text.len() || !is_ident_byte(text.as_bytes()[pos + word.len()]))
}

fn parse_wf_fn_prefix(fn_name: &str) -> Option<u8> {
    let rest = fn_name.strip_prefix("wf")?;
    let suffix = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let digits = rest.strip_suffix(suffix)?;
    (!digits.is_empty() && suffix.starts_with('_')).then(|| digits.parse::<u8>().ok()).flatten().filter(|i| (1..=TOTAL_WORKFLOWS_U8).contains(i))
}

fn has_wf_start(body: &str, idx: u8) -> bool {
    let c: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    c.contains(&format!("WorkflowRun::start(\"WF-{idx:02}\"")) || c.contains(&format!("WorkflowRun::start_with_clipboard(\"WF-{idx:02}\""))
}

fn has_finish(body: &str) -> bool {
    let compact: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if let Some(var) = find_workflow_run_binding(body) {
        return compact.contains(&format!("{var}.finish()")) || compact.contains(&format!("WorkflowRun::finish({var})"));
    }
    compact.match_indices("WorkflowRun::start").any(|(pos, _)| {
        compact
            .get(pos..)
            .and_then(|s| s.find('('))
            .and_then(|o| find_matching_delim(compact.as_bytes(), pos + o, b'(', b')'))
            .and_then(|c| compact.get(c + 1..))
            .is_some_and(|a| a.starts_with(".finish()") || a.starts_with("?.finish()"))
    })
}

fn find_workflow_run_binding(code: &str) -> Option<String> {
    for (i, _) in code.match_indices("let").filter(|&(i, _)| is_word_at(code, i, "let")) {
        let rest = code.get(i + 3..)?.trim_start();
        let after_mut = rest.strip_prefix("mut").filter(|r| r.starts_with(char::is_whitespace)).map_or(rest, str::trim_start);
        let ident = after_mut.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or("");
        if let Some((lhs, rhs)) = after_mut.split_once('=')
            && !ident.is_empty()
            && !lhs.contains(';')
            && rhs.chars().take(64).filter(|c| !c.is_ascii_whitespace()).collect::<String>().starts_with("WorkflowRun::start")
        {
            return Some(ident.to_owned());
        }
    }
    None
}

fn has_guarded_find_by_widget_id(code: &str) -> bool {
    let bytes = code.as_bytes();
    for (kw, allow_let) in [("if", true), ("while", true), ("match", false)] {
        for (i, _) in code.match_indices(kw).filter(|&(i, _)| is_word_at(code, i, kw)) {
            let trimmed = code.get(i + kw.len()..).unwrap_or("").trim_start();
            let start = code.len() - trimmed.len();
            let (mut past_eq, mut p, mut b, mut pb) = (!(allow_let && is_word_at(code, start, "let")), 0usize, 0usize, 0usize);
            for (rel, ch) in trimmed.char_indices() {
                if trimmed.get(rel..).is_some_and(|s| s.starts_with("find_by_widget_id(")) {
                    return true;
                }
                let j = start + rel;
                match ch {
                    '(' => p += 1,
                    ')' => p = p.saturating_sub(1),
                    '[' => b += 1,
                    ']' => b = b.saturating_sub(1),
                    ';' if p == 0 && b == 0 => break,
                    '=' if !past_eq
                        && p == 0
                        && b == 0
                        && pb == 0
                        && !matches!(j.checked_sub(1).map(|k| bytes[k]), Some(b'!' | b'<' | b'>' | b'='))
                        && !matches!(bytes.get(j + 1), Some(b'=' | b'>')) =>
                    {
                        past_eq = true
                    }
                    '{' if !past_eq => pb += 1,
                    '}' if !past_eq => pb = pb.saturating_sub(1),
                    '{' if past_eq && p == 0 && b == 0 => break,
                    _ => {}
                }
            }
        }
    }
    false
}

fn check_workflow_run_lint(path: &str, f: &ScannedRustFn<'_>, violations: &mut Vec<EvidenceViolation>) {
    for tok in ["execute_command", "app_mut", "state_mut", "dispatch_gui_action"] {
        if f.body_code_only.match_indices(tok).any(|(pos, _)| is_word_at(f.body_code_only, pos, tok)) {
            push_ev(violations, path, f.line, format!("`WorkflowRun` test `{}` contains forbidden bypass `{tok}`", f.name));
        }
    }
    if f.body_code_only.chars().filter(|c| !c.is_ascii_whitespace()).collect::<String>().contains("Session::from_document") {
        push_ev(violations, path, f.line, format!("`WorkflowRun` test `{}` contains forbidden seeded bypass `Session::from_document`", f.name));
    }
    if has_guarded_find_by_widget_id(f.body_code_only) {
        push_ev(violations, path, f.line, format!("`WorkflowRun` test `{}` guards a UI step on `if`/`if let` `find_by_widget_id(`", f.name));
    }
}

fn parse_shard_wf_range(shard_cell: &str) -> Option<(u8, u8)> {
    const SHARDS: &[(&str, u8, u8)] = &[
        ("shard 1", 1, 6),
        ("shard 2", 7, 15),
        ("shard 3", 16, 23),
        ("shard 4", 24, 28),
        ("shard 5", 29, 36),
        ("shard 6", 37, 43),
        ("shard 7", 44, 49),
        ("shard 8", 51, 58),
        ("shard 9", 59, 65),
        ("baseline", 1, 50),
        ("extended", 51, 65),
    ];
    let lower = shard_cell.to_ascii_lowercase();
    let nums: Vec<u8> = lower
        .match_indices("wf-")
        .filter_map(|(i, _)| {
            lower.get(i + 3..)?.split(|c: char| !c.is_ascii_digit()).next()?.parse::<u8>().ok().filter(|v| (1..=TOTAL_WORKFLOWS_U8).contains(v))
        })
        .collect();
    if nums.len() >= 2 && nums[0] <= nums[1] {
        Some((nums[0], nums[1]))
    } else if nums.len() == 1 {
        Some((nums[0], nums[0]))
    } else {
        SHARDS.iter().find_map(|&(pat, s, e)| lower.contains(pat).then_some((s, e)))
    }
}

fn parse_labeled_fraction(text: &str, label: &str) -> Vec<(usize, usize)> {
    let (lower, mut out) = (text.to_ascii_lowercase(), Vec::new());
    let bytes = lower.as_bytes();
    for (pos, _) in lower.match_indices(label) {
        if pos > 0 && bytes[pos - 1].is_ascii_alphanumeric() {
            continue;
        }
        let rest =
            lower.get(pos + label.len()..).unwrap_or("").trim_start_matches(|c: char| c.is_ascii_whitespace() || matches!(c, '`' | '*' | '_' | ':'));
        if let Some((n_s, after)) = rest.split_once('/')
            && let (Ok(n), Ok(d)) = (n_s.trim().parse(), after.trim_start().split(|c: char| !c.is_ascii_digit()).next().unwrap_or("").parse())
        {
            out.push((n, d));
        }
    }
    out
}

fn validate_section62_completion_counts(
    path: &str,
    line: usize,
    shard_cell: &str,
    human_cell: &str,
    complete_wfs: &BTreeSet<u8>,
    violations: &mut Vec<EvidenceViolation>,
) {
    let (c_claims, fs_claims) = (parse_labeled_fraction(human_cell, "complete"), parse_labeled_fraction(human_cell, "five-stage"));
    if c_claims.is_empty() && fs_claims.is_empty() {
        return;
    }
    let Some((start, end)) = parse_shard_wf_range(shard_cell) else {
        return push_ev(violations, path, line, format!("cannot resolve `WF` shard range for §6.2 row `{}`", shard_cell.trim()));
    };
    let (expected_m, actual) = (usize::from(end - start + 1), (start..=end).filter(|i| complete_wfs.contains(i)).count());
    for (label, claims) in [("complete", c_claims), ("five-stage", fs_claims)] {
        for (n, m) in claims.into_iter().filter(|&(n, m)| m != expected_m || n != actual) {
            push_ev(
                violations,
                path,
                line,
                format!(
                    "§6.2 row claims `{label} {n} / {m}` for `WF-{start:02}`..`WF-{end:02}`, but statically counted `{actual} / {expected_m}` five-stage complete tests"
                ),
            );
        }
    }
}

#[must_use]
pub fn check_ui_parity_five_stage_claims(path: &str, ui_md: &str, complete_wfs: &BTreeSet<u8>) -> Vec<EvidenceViolation> {
    let (mut violations, mut fence) = (Vec::new(), None);
    for (idx, line) in ui_md.lines().enumerate() {
        let (t, lower) = (line.trim(), line.trim().to_ascii_lowercase());
        if update_fence(t, &mut fence) || fence.is_some() || !is_checked_markdown_item(t) || lower.contains(AGENT_HARNESS_TAG) {
            continue;
        }
        let (cf, fsf) = (parse_labeled_fraction(&lower, "complete"), parse_labeled_fraction(&lower, "five-stage"));
        if !(lower.contains("five-stage") || lower.contains("five stage") || !cf.is_empty()) {
            continue;
        }
        let (start, end) = if lower.contains("wf-01") && lower.contains("wf-50") {
            (1, 50)
        } else if lower.contains("wf-51") && lower.contains("wf-65") {
            (51, 65)
        } else {
            (1, TOTAL_WORKFLOWS_U8)
        };
        let (expected_m, actual) = (usize::from(end - start + 1), (start..=end).filter(|i| complete_wfs.contains(i)).count());
        let mut claims: Vec<usize> = cf.into_iter().chain(fsf).map(|(n, _)| n).collect();
        claims.extend(parse_workflow_fractions(&lower).into_iter().filter(|&(_, d, _)| d == expected_m || d == TOTAL_WORKFLOWS).map(|(n, _, _)| n));
        if if claims.is_empty() { actual < expected_m } else { claims.iter().any(|&n| n > actual) } {
            push_ev(
                &mut violations,
                path,
                idx + 1,
                format!(
                    "checked `[x]` item claims five-stage completion beyond counted `WorkflowRun` tests (`{actual}/{expected_m}` complete in `WF-{start:02}`..`WF-{end:02}`)"
                ),
            );
        }
    }
    violations
}

#[must_use]
pub fn check_markdown_tables(path: &str, md: &str) -> (usize, Vec<DuplicateKey>) {
    let lines: Vec<&str> = md.lines().collect();
    let (mut tables, mut dups, mut seen, mut fence, mut in_table, mut idx) = (0usize, Vec::new(), BTreeSet::new(), false, false, 0usize);
    while let Some(&raw) = lines.get(idx) {
        let t = raw.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            (fence, in_table) = (!fence, false);
            seen.clear();
        } else if !fence && !in_table {
            if is_table_row(t) && lines.get(idx + 1).is_some_and(|n| is_table_separator(n.trim())) {
                (tables, in_table) = (tables + 1, true);
                seen.clear();
                idx += 2;
                continue;
            }
        } else if !fence {
            if is_table_row(t) {
                if let Some(col1) = parse_first_cell(t) {
                    let norm = normalize_key(&col1);
                    if !norm.is_empty() && !seen.insert(norm) {
                        dups.push(DuplicateKey { path: path.to_owned(), line: idx + 1, key: col1 });
                    }
                }
            } else {
                (in_table, ()) = (false, seen.clear());
            }
        }
        idx += 1;
    }
    (tables, dups)
}

fn is_table_row(t: &str) -> bool {
    t.starts_with('|') && t.len() > 1
}
fn is_table_separator(t: &str) -> bool {
    is_table_row(t) && {
        let cells = split_table_cells(t);
        !cells.is_empty()
            && cells.iter().all(|c| {
                let c = c.trim();
                !c.is_empty() && c.contains('-') && c.bytes().all(|b| matches!(b, b'-' | b':' | b' '))
            })
    }
}

fn parse_first_cell(row: &str) -> Option<String> {
    let (mut chars, mut cell) = (row.trim().strip_prefix('|')?.chars(), String::new());
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(n) = chars.next() {
                    cell.push(n);
                }
            }
            '|' => break,
            _ => cell.push(ch),
        }
    }
    Some(cell.trim().to_owned())
}

fn normalize_key(raw: &str) -> String {
    let mut s = raw.trim();
    loop {
        let next = s
            .strip_prefix("**")
            .and_then(|v| v.strip_suffix("**"))
            .or_else(|| s.strip_prefix("__").and_then(|v| v.strip_suffix("__")))
            .or_else(|| s.strip_prefix('`').and_then(|v| v.strip_suffix('`')))
            .map_or(s, str::trim);
        if next == s {
            break;
        }
        s = next;
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}

#[must_use]
pub fn check_live_status_metrics(root: &Path, status_md: &str) -> Vec<EvidenceViolation> {
    let (mut violations, in_ci) = (Vec::new(), crate::workflows::is_ci_env());
    #[cfg(feature = "docs-site")]
    let live_engine = Some(curvebook_engine::cmd::registry().len());
    #[cfg(not(feature = "docs-site"))]
    let live_engine: Option<usize> = {
        if in_ci {
            push_ev(
                &mut violations,
                STATUS_REL,
                1,
                "xtask built without `--features docs-site` under CI; cannot verify live engine command count parity",
            );
        } else if !cfg!(test) {
            eprintln!("status: warning: xtask built without `--features docs-site`; skipping live engine command count parity checks");
        }
        None
    };
    let _ = in_ci;
    let ui_rel = "crates/ui-egui/src/menus/ui_commands.rs";
    let live_ui = match fs::read_to_string(root.join(ui_rel))
        .map_err(|e| format!("cannot read `{ui_rel}`: {e}"))
        .and_then(|s| count_ui_command_specs_in_source(&s).ok_or_else(|| format!("cannot parse `pub const UI_COMMANDS` in `{ui_rel}`")))
    {
        Ok(n) => Some(n),
        Err(err) => {
            push_ev(&mut violations, ui_rel, 1, err);
            None
        }
    };
    let live_pkgs = crate::layers::TABLE.len();
    for h in ["## Verified Capabilities & Headline Numbers", "## Known Limits"] {
        if !status_md.lines().any(|l| l.trim() == h) {
            push_ev(&mut violations, STATUS_REL, 1, format!("missing required section heading `{h}`"));
        }
    }
    let (mut saw_engine, mut saw_ui, mut saw_plat) = (false, false, false);
    for (idx, raw) in status_md.lines().enumerate() {
        let t = raw.trim();
        if !is_table_row(t) || is_table_separator(t) {
            continue;
        }
        let Some(col1) = parse_first_cell(t) else { continue };
        let norm = normalize_key(&col1).replace('`', "");
        if norm == "engine command registry" {
            saw_engine = true;
            if let Some(n) = live_engine
                && !cell_contains_number(t, &n.to_string())
            {
                push_ev(&mut violations, STATUS_REL, idx + 1, format!("Engine command registry row does not report live count `{n}`"));
            }
        } else if norm == "ui command catalog" {
            saw_ui = true;
            if let Some(n) = live_ui
                && !cell_contains_number(t, &n.to_string())
            {
                push_ev(&mut violations, STATUS_REL, idx + 1, format!("UI command catalog row does not report live count `{n}`"));
            }
        } else if norm.starts_with("platform, r0 alignment") {
            saw_plat = true;
            if !t.contains(&format!("{live_pkgs}` packages")) && !t.contains(&format!("{live_pkgs} packages")) {
                push_ev(&mut violations, STATUS_REL, idx + 1, format!("Platform/R0 row does not report live `{live_pkgs}` workspace packages"));
            }
        }
    }
    for (ok, label) in
        [(saw_engine, "Engine command registry"), (saw_ui, "UI command catalog"), (saw_plat, "Platform, `R0` alignment & Zero Telemetry")]
    {
        if !ok {
            push_ev(&mut violations, STATUS_REL, 1, format!("missing `{label}` row in Verified Capabilities table"));
        }
    }
    check_doc_count_parity(root, live_engine, live_ui, &mut violations);
    violations
}

fn count_ui_command_specs_in_source(src: &str) -> Option<usize> {
    let (code_only, _) = blank_comments_and_literals_both(src, false);
    let after = code_only.split_once("pub const UI_COMMANDS:")?.1.split_once('=')?.1;
    let slice = after.get(after.find('[')?..)?;
    let body = slice.get(..find_matching_delim(slice.as_bytes(), 0, b'[', b']')?)?;
    Some(
        body.match_indices("UiCommandSpec")
            .filter(|&(p, _)| is_word_at(body, p, "UiCommandSpec") && body.get(p + 13..).is_some_and(|r| r.trim_start().starts_with('{')))
            .count(),
    )
}

fn cell_contains_number(row: &str, needle: &str) -> bool {
    let cell = row.split('|').nth(2).unwrap_or(row);
    let bytes = cell.as_bytes();
    cell.match_indices(needle)
        .any(|(p, _)| (p == 0 || !bytes[p - 1].is_ascii_digit()) && bytes.get(p + needle.len()).is_none_or(|b| !b.is_ascii_digit()))
}

fn read_doc(root: &Path, rel: &str, violations: &mut Vec<EvidenceViolation>) -> Option<String> {
    fs::read_to_string(root.join(rel)).map_err(|e| push_ev(violations, rel, 1, format!("cannot read `{rel}`: {e}"))).ok()
}

fn check_doc_count_parity(root: &Path, live_engine: Option<usize>, live_ui: Option<usize>, violations: &mut Vec<EvidenceViolation>) {
    if let Some(doc) = read_doc(root, "docs/commands.md", violations)
        && let Some(n) = live_engine
        && !doc.contains(&format!("Registered commands: {n}"))
    {
        push_ev(violations, "docs/commands.md", 1, format!("expected `Registered commands: {n}` matching live registry"));
    }
    if let Some(doc) = read_doc(root, "AGENTS.md", violations) {
        if let Some(n) = live_engine
            && !doc.contains(&format!("`{n}` commands"))
        {
            push_ev(violations, "AGENTS.md", 1, format!("expected `` `{n}` commands `` matching live command registry"));
        }
        if let Some(n) = live_ui
            && !doc.contains(&format!("`{n}` entries"))
        {
            push_ev(violations, "AGENTS.md", 1, format!("expected `` `{n}` entries `` matching live `UI_COMMANDS` count"));
        }
    }
    if let Some(doc) = read_doc(root, "docs/control-protocol.md", violations)
        && let Some(n) = live_ui
    {
        let exact = format!("`{n}` synced `UI_COMMANDS`");
        if doc.contains("`32` synced `UI_COMMANDS`") || (!doc.contains("all synced `UI_COMMANDS`") && !doc.contains(&exact)) {
            push_ev(violations, "docs/control-protocol.md", 1, format!("expected `all synced `UI_COMMANDS`` or `{exact}`"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_tests(names: &[&str]) -> WorkspaceTests {
        let mut t = WorkspaceTests::default();
        for &q in names {
            t.qualified.insert(q.to_owned());
            if let Some(u) = q.rsplit("::").next() {
                t.unqualified.insert(u.to_owned());
            }
        }
        t
    }

    #[test]
    fn ui_evidence_section62_telemetry_and_static_wf_rules() {
        let tests = mk_tests(&["desktop_host_loop::file_drop_human_path", "human_wf_01_06::wf01_human_xy_demo", "gui_wf_foundations::wf01_t_test"]);
        assert!(check_ui_parity_evidence(UI_EVIDENCE_REL, "- [x] Drop (`desktop_host_loop::file_drop_human_path`).\n- [x] Save — manual: 2026-11-16 macOS.\n- [x] *(agent harness)* (`--control`).\n", Some(&tests)).is_empty());
        assert_eq!(check_ui_parity_evidence(UI_EVIDENCE_REL, "- [x] No evidence.\n", None).len(), 1);
        assert!(
            check_ui_parity_evidence(UI_EVIDENCE_REL, "- [x] Bad (`gui_wf_foundations::wf01_t_test`).\n", Some(&tests))[0]
                .reason
                .contains("agent-harness")
        );
        assert_eq!(check_ui_parity_evidence(UI_EVIDENCE_REL, "- [x] Save — manual: todo.\n", None).len(), 1);

        let all_65_names: Vec<String> = (1..=65).map(|i| format!("human_wf_demo::wf{i:02}_human_ex")).collect();
        let all_65 = mk_tests(&all_65_names.iter().map(String::as_str).collect::<Vec<_>>());
        let (tel, v) = check_backend_workflow_telemetry(UI_EVIDENCE_REL, "- [x] All — `65/65 egui`\n", &all_65);
        assert!(v.is_empty() && tel.egui_claimed == 65 && tel.egui_backed == 65);

        let mut scanned = WorkspaceTests::default();
        scan_tests_in_rust_source("s", None, "#[test] #[ignore] fn skip() {}\n#[test] fn active_human_test() {}\n", &mut scanned);
        assert!(scanned.contains_test("s::active_human_test") && !scanned.unqualified.contains("skip"));

        let wf_src = "#[test] fn wf01_human_ok() { let mut r = WorkflowRun::start(\"WF-01\"); r.finish() }\n#[test] fn wf02_human_bad() { let mut r = WorkflowRun::start(\"WF-02\"); if r.frame_meta().find_by_widget_id(\"a\").is_some() {} let _ = r.app_mut(); }\n";
        let (comp, lints) = analyze_human_wf_sources(&[("crates/ui-egui/tests/human_wf_01_06.rs", wf_src)]);
        assert!(
            comp == BTreeSet::from([1u8])
                && lints.iter().any(|x| x.reason.contains("find_by_widget_id("))
                && lints.iter().any(|x| x.reason.contains("app_mut"))
        );

        assert!(check_gui_workflows_human_path_with_completion(GUI_WORKFLOWS_REL, "### 6.2 Status\n| Shard | Agent | Human path |\n|---|---|---|\n| Shard 1 (`WF-01`–`WF-06`) | Pass | complete `1 / 6` (`human_wf_01_06::wf01_human_xy_demo`) |\n", Some(&tests), Some(&comp)).is_empty());
        assert!(!check_ui_parity_five_stage_claims(UI_EVIDENCE_REL, "- [x] Five-stage `WF-01`–`WF-50` complete.\n", &comp).is_empty());
        assert_eq!(
            check_markdown_tables(ROADMAP_REL, "| K | V |\n|---|---|\n| `R0.3` | a |\n| `R0.3` | b |\n"),
            (1, vec![DuplicateKey { path: ROADMAP_REL.into(), line: 4, key: "`R0.3`".into() }])
        );

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let live_violations = check_live_status_metrics(root, &std::fs::read_to_string(root.join(STATUS_REL)).unwrap());
        #[cfg(not(feature = "docs-site"))]
        if crate::workflows::is_ci_env() {
            assert_eq!(live_violations.len(), 1, "expected only the CI docs-site guard violation: {live_violations:?}");
            assert!(live_violations[0].reason.contains("--features docs-site"), "{live_violations:?}");
        } else {
            assert!(live_violations.is_empty(), "{live_violations:?}");
        }
        #[cfg(feature = "docs-site")]
        assert!(live_violations.is_empty(), "{live_violations:?}");
        assert_eq!(count_ui_command_specs_in_source("pub const UI_COMMANDS: &[UiCommandSpec] = &[UiCommandSpec\n{ id: \"a\" }];\n"), Some(1));
    }
}
