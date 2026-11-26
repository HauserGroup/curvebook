//! `cargo xtask status`: verify unique first-column keys in `docs/STATUS.md` and `docs/ROADMAP.md`,
//! and enforce test-evidence gates across `docs/evidence/user-interface.md` and `docs/gui-workflows.md` §6.2
//! (Pitfall #6: semantic agent-harness tests/flags never prove human GUI parity).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::scan_util::{format_error_list, has_ext_ci, is_real_dir, rel_slash_lossy, walk_file_paths};

const STATUS_REL: &str = "docs/STATUS.md";
const ROADMAP_REL: &str = "docs/ROADMAP.md";
const UI_EVIDENCE_REL: &str = "docs/evidence/user-interface.md";
const GUI_WORKFLOWS_REL: &str = "docs/gui-workflows.md";
const AGENT_HARNESS_TAG: &str = "*(agent harness)*";

const AGENT_HARNESS_MARKERS: &[&str] = &[
    "--control",
    "--event-script",
    "--gui-only",
    "dispatch_gui_action",
    "dispatch_control_line",
    "run_event_script_jsonl",
    "ui.invoke_menu",
    "ui.answer_dialog",
    "ui.set_field",
    "ui.click",
    "ui.press_key",
    "ui.inspect",
    "ui.screenshot",
    "ui.pointer_gesture",
    "ui.type_text",
    "ui.switch_tab",
    "ui_inspect",
    "ui_screenshot",
    "ui_pointer_gesture",
    "ui_type_text",
    "ui_press_key",
    "ui_invoke_menu",
    "ui_switch_tab",
    "ui_answer_dialog",
    "gui_only_workflows",
    "gui_w22b_qol_and_r9",
    "gui_only_surface",
    "mcp_gui_only_parity",
    "gui_only_wiring",
    "jsonl_script_playback",
    "control_loopback",
    "control_auth_and_panic",
];

const HUMAN_DRIVER_TOKENS: &[&str] = &["humanpathdriver", "eguihumanpathdriver", "egui_kittest"];

const KNOWN_BACKENDS: &[&str] = &["legacy", "egui"];
const TOTAL_WORKFLOWS: usize = 65;
const TOTAL_WORKFLOWS_U8: u8 = 65;

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

/// Violation emitted when a `[x]` UI claim or non-zero human-path entry lacks qualifying
/// human-path test or recorded computer-use / manual verification evidence.
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

/// Index of non-ignored `#[test]` function names collected from `crates/` and `apps/`.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceTests {
    pub unqualified: BTreeSet<String>,
    pub qualified: BTreeSet<String>,
}

impl WorkspaceTests {
    /// Returns `true` if `token` names a discovered workspace test.
    #[must_use]
    pub fn contains_test(&self, token: &str) -> bool {
        if token.contains("::") { self.qualified.contains(token) } else { self.unqualified.contains(token) }
    }
}

/// Validate Column-1 key uniqueness across `docs/STATUS.md` and `docs/ROADMAP.md`
/// and enforce `[x]` / human-path evidence across `docs/evidence/user-interface.md` and `docs/gui-workflows.md`.
pub fn run(root: &Path) -> Result<(), String> {
    let mut total_tables = 0usize;
    let mut duplicates = Vec::new();

    for rel in [STATUS_REL, ROADMAP_REL] {
        let text = fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let (tables, mut dups) = check_markdown_tables(rel, &text);
        if tables == 0 {
            return Err(format!("{rel}: expected at least one Markdown table"));
        }
        total_tables += tables;
        duplicates.append(&mut dups);
    }

    let status_text = fs::read_to_string(root.join(STATUS_REL)).map_err(|e| format!("{STATUS_REL}: {e}"))?;
    let mut violations = check_live_status_metrics(root, &status_text);

    let workspace_tests = collect_workspace_tests(root)?;
    let (complete_wfs, mut wf_violations) = scan_human_wf_completion(root)?;
    let ui_parity_text = fs::read_to_string(root.join(UI_EVIDENCE_REL)).map_err(|e| format!("{UI_EVIDENCE_REL}: {e}"))?;
    violations.extend(check_ui_parity_evidence(UI_EVIDENCE_REL, &ui_parity_text, Some(&workspace_tests)));
    violations.extend(check_ui_parity_five_stage_claims(UI_EVIDENCE_REL, &ui_parity_text, &complete_wfs));
    let (telemetry, telemetry_violations) = check_backend_workflow_telemetry(UI_EVIDENCE_REL, &ui_parity_text, &workspace_tests);
    violations.extend(telemetry_violations);

    let workflows_text = fs::read_to_string(root.join(GUI_WORKFLOWS_REL)).map_err(|e| format!("{GUI_WORKFLOWS_REL}: {e}"))?;
    violations.extend(check_gui_workflows_human_path_with_completion(
        GUI_WORKFLOWS_REL,
        &workflows_text,
        Some(&workspace_tests),
        Some(&complete_wfs),
    ));
    violations.append(&mut wf_violations);

    if duplicates.is_empty() && violations.is_empty() {
        println!(
            "status: verified {total_tables} Markdown table(s) across {STATUS_REL} and {ROADMAP_REL}, \
             live metrics, [x] evidence in {UI_EVIDENCE_REL}, and §6.2 Human path column in {GUI_WORKFLOWS_REL} \
             ({}/65 egui; backed human tests: {}/65 egui; five-stage complete: {}/65)",
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

/// Collect all non-ignored `#[test]` functions from `crates/` and `apps/` under `root`.
pub fn collect_workspace_tests(root: &Path) -> Result<WorkspaceTests, String> {
    let mut tests = WorkspaceTests::default();
    let mut paths = Vec::new();
    for dir_name in ["crates", "apps"] {
        let base = root.join(dir_name);
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
        let parent = if matches!(stem, "mod" | "lib" | "main") { path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) } else { None };
        scan_tests_in_rust_source(stem, parent, &src, &mut tests);
    }
    Ok(tests)
}

fn scan_tests_in_rust_source(stem: &str, parent_mod: Option<&str>, src: &str, out: &mut WorkspaceTests) {
    let (_with_strings, code_only) = build_stripped_views(src);
    let mut pending_test = false;
    let mut pending_ignored = false;
    let mut attr_depth = 0isize;

    for line in code_only.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if attr_depth > 0 {
            attr_depth += bracket_delta(trimmed);
            continue;
        }
        if trimmed.starts_with("#[") {
            let mut rest = trimmed;
            while rest.starts_with("#[") {
                if let Some((attr, tail)) = split_balanced_attr(rest) {
                    pending_test |= is_test_attribute(attr);
                    pending_ignored |= is_ignore_attribute(attr);
                    rest = tail.trim_start();
                } else {
                    pending_test |= is_test_attribute(rest);
                    pending_ignored |= is_ignore_attribute(rest);
                    attr_depth += bracket_delta(rest);
                    rest = "";
                    break;
                }
            }
            if !rest.is_empty() {
                if pending_test
                    && !pending_ignored
                    && let Some(name) = extract_fn_name(rest)
                {
                    record_test_name(stem, parent_mod, name, out);
                }
                pending_test = false;
                pending_ignored = false;
            }
            continue;
        }
        if pending_test
            && !pending_ignored
            && let Some(name) = extract_fn_name(trimmed)
        {
            record_test_name(stem, parent_mod, name, out);
        }
        pending_test = false;
        pending_ignored = false;
    }
}

fn split_balanced_attr(input: &str) -> Option<(&str, &str)> {
    let mut depth = 0isize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    let split = idx + ch.len_utf8();
                    return input.split_at_checked(split);
                }
            }
            _ => {}
        }
    }
    None
}

fn record_test_name(stem: &str, parent_mod: Option<&str>, name: &str, out: &mut WorkspaceTests) {
    out.unqualified.insert(name.to_owned());
    if !stem.is_empty() {
        out.qualified.insert(format!("{stem}::{name}"));
    }
    if let Some(parent) = parent_mod
        && !parent.is_empty()
    {
        out.qualified.insert(format!("{parent}::{name}"));
    }
}

fn bracket_delta(line: &str) -> isize {
    line.chars().fold(0isize, |acc, ch| match ch {
        '[' => acc + 1,
        ']' => acc - 1,
        _ => acc,
    })
}

fn is_test_attribute(attr: &str) -> bool {
    attr.starts_with("#[test]") || attr.contains("::test]") || attr.contains("::test(")
}

fn is_ignore_attribute(attr: &str) -> bool {
    attr.starts_with("#[ignore")
}

fn extract_fn_name(trimmed: &str) -> Option<&str> {
    let (prefix_raw, after_fn_raw) = trimmed.split_once("fn ")?;
    let prefix = prefix_raw.trim();
    let valid_prefix =
        prefix.is_empty() || prefix.split_whitespace().all(|kw| kw == "async" || kw == "unsafe" || kw == "const" || kw.starts_with("pub"));
    if !valid_prefix {
        return None;
    }
    let after_fn = after_fn_raw.trim_start();
    let name = after_fn.split(['(', '<', ' ']).next()?;
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { Some(name) } else { None }
}

/// Enforce test or manual-run evidence on every `[x]` item/row in `docs/evidence/user-interface.md`
/// excluding rows/items marked `*(agent harness)*`.
#[must_use]
pub fn check_ui_parity_evidence(path: &str, md: &str, known_tests: Option<&WorkspaceTests>) -> Vec<EvidenceViolation> {
    let mut violations = Vec::new();
    let mut fence: Option<char> = None;

    for (idx, line) in md.lines().enumerate() {
        let trimmed = line.trim();
        if update_fence(trimmed, &mut fence) || fence.is_some() {
            continue;
        }
        if !is_checked_markdown_item(trimmed) || line.to_ascii_lowercase().contains(AGENT_HARNESS_TAG) {
            continue;
        }
        if let Some(violation) = validate_human_evidence(path, idx + 1, line, known_tests) {
            violations.push(violation);
        }
    }

    violations
}

/// Enforce evidence requirements for the `Human path` column of `docs/gui-workflows.md` §6.2.
/// Validate `docs/gui-workflows.md` §6.2 Human path column evidence against `known_tests`.
#[cfg(test)]
#[must_use]
pub fn check_gui_workflows_human_path(path: &str, md: &str, known_tests: Option<&WorkspaceTests>) -> Vec<EvidenceViolation> {
    check_gui_workflows_human_path_with_completion(path, md, known_tests, None)
}

/// Validate `docs/gui-workflows.md` §6.2 Human path column evidence and optional `WorkflowRun` `complete N / M` counts.
#[must_use]
pub fn check_gui_workflows_human_path_with_completion(
    path: &str,
    md: &str,
    known_tests: Option<&WorkspaceTests>,
    complete_wfs: Option<&BTreeSet<u8>>,
) -> Vec<EvidenceViolation> {
    let mut violations = Vec::new();
    let mut state = Section62State::default();

    for (idx, line) in md.lines().enumerate() {
        let trimmed = line.trim();
        if update_fence(trimmed, &mut state.fence) || state.fence.is_some() {
            continue;
        }
        state.step(path, idx + 1, trimmed, known_tests, complete_wfs, &mut violations);
    }

    if !state.verified_table {
        violations.push(EvidenceViolation { path: path.to_owned(), line: 1, reason: String::from("missing §6.2 table with 'Human path' column") });
    }

    violations
}

#[derive(Default)]
struct Section62State {
    fence: Option<char>,
    in_section: bool,
    human_col: Option<usize>,
    saw_separator: bool,
    verified_table: bool,
}

impl Section62State {
    fn step(
        &mut self,
        path: &str,
        line_no: usize,
        trimmed: &str,
        known_tests: Option<&WorkspaceTests>,
        complete_wfs: Option<&BTreeSet<u8>>,
        violations: &mut Vec<EvidenceViolation>,
    ) {
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("### 6.2") {
            self.in_section = true;
            self.reset_table();
            return;
        }
        if self.in_section && (trimmed.starts_with("## ") || trimmed.starts_with("### ")) {
            self.in_section = false;
            self.reset_table();
            return;
        }
        if !self.in_section || !is_table_row(trimmed) {
            if self.human_col.is_some() && !trimmed.is_empty() {
                self.reset_table();
            }
            return;
        }
        self.visit_table_row(path, line_no, trimmed, known_tests, complete_wfs, violations);
    }

    fn reset_table(&mut self) {
        self.human_col = None;
        self.saw_separator = false;
    }

    fn visit_table_row(
        &mut self,
        path: &str,
        line_no: usize,
        trimmed: &str,
        known_tests: Option<&WorkspaceTests>,
        complete_wfs: Option<&BTreeSet<u8>>,
        violations: &mut Vec<EvidenceViolation>,
    ) {
        let cells = split_table_cells(trimmed);
        let Some(col_idx) = self.human_col else {
            self.human_col =
                cells.iter().position(|c| c.trim().trim_matches(|ch: char| matches!(ch, '*' | '_' | '`' | ' ')).eq_ignore_ascii_case("human path"));
            self.saw_separator = false;
            if self.human_col.is_some() {
                self.verified_table = true;
            }
            return;
        };
        if !self.saw_separator && is_table_separator(trimmed) {
            self.saw_separator = true;
            return;
        }
        if let Some(cell) = cells.get(col_idx) {
            if human_path_requires_evidence(cell)
                && let Some(violation) = validate_human_evidence(path, line_no, cell, known_tests)
            {
                violations.push(violation);
            }
            if let Some(complete_set) = complete_wfs {
                let shard_cell = cells.first().copied().unwrap_or("");
                validate_section62_completion_counts(path, line_no, shard_cell, cell, complete_set, violations);
            }
        }
    }
}

fn update_fence(trimmed: &str, fence: &mut Option<char>) -> bool {
    let fence_char = if trimmed.starts_with("```") {
        Some('`')
    } else if trimmed.starts_with("~~~") {
        Some('~')
    } else {
        None
    };
    let Some(ch) = fence_char else {
        return false;
    };
    match *fence {
        Some(active) if active == ch => *fence = None,
        None => *fence = Some(ch),
        _ => {}
    }
    true
}

fn is_checked_markdown_item(trimmed: &str) -> bool {
    if is_table_row(trimmed) {
        return split_table_cells(trimmed).into_iter().any(|cell| {
            let c = cell.trim();
            c == "[x]" || c == "[X]" || c.starts_with("[x] ") || c.starts_with("[X] ")
        });
    }

    let after_marker = if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("+ ")) {
        Some(rest.trim_start())
    } else if let Some((prefix, rest)) = trimmed.split_once(['.', ')']) {
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) { rest.strip_prefix(' ').map(str::trim_start) } else { None }
    } else {
        None
    };

    after_marker.is_some_and(|rest| rest == "[x]" || rest == "[X]" || rest.starts_with("[x] ") || rest.starts_with("[X] "))
}

fn split_table_cells(trimmed: &str) -> Vec<&str> {
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').collect()
}

fn human_path_requires_evidence(cell: &str) -> bool {
    let clean = cell.trim().trim_matches(|c: char| c == '`' || c == '*' || c == '_' || c.is_whitespace());
    let lower = clean.to_ascii_lowercase();
    if matches!(lower.as_str(), "" | "—" | "-" | "–" | "n/a" | "no" | "none" | "todo" | "pending" | "untested" | "[ ]" | "0" | "0%") {
        return false;
    }
    if has_invalid_backend_fraction(&lower) {
        return true;
    }
    !all_fractions_are_zero(&lower)
}

fn parse_workflow_fractions(text: &str) -> Vec<(usize, usize, Option<&str>)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    for (slash_idx, &b) in bytes.iter().enumerate() {
        if b != b'/' {
            continue;
        }
        let mut left_end = slash_idx;
        while left_end > 0 && bytes[left_end - 1].is_ascii_whitespace() {
            left_end -= 1;
        }
        let mut left_start = left_end;
        while left_start > 0 && bytes[left_start - 1].is_ascii_digit() {
            left_start -= 1;
        }
        if left_start == left_end || (left_start > 0 && bytes[left_start - 1].is_ascii_alphanumeric()) {
            continue;
        }

        let mut right_start = slash_idx + 1;
        while right_start < bytes.len() && bytes[right_start].is_ascii_whitespace() {
            right_start += 1;
        }
        let mut right_end = right_start;
        while right_end < bytes.len() && bytes[right_end].is_ascii_digit() {
            right_end += 1;
        }
        if right_start == right_end || (right_end < bytes.len() && bytes[right_end].is_ascii_alphanumeric()) {
            continue;
        }

        let Some(num) = text.get(left_start..left_end).and_then(|s| s.parse::<usize>().ok()) else {
            continue;
        };
        let Some(den) = text.get(right_start..right_end).and_then(|s| s.parse::<usize>().ok()) else {
            continue;
        };
        let trailing = text.get(right_end..).unwrap_or("").trim_start().split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("");
        let backend = KNOWN_BACKENDS.iter().copied().find(|&b_name| trailing.eq_ignore_ascii_case(b_name));
        out.push((num, den, backend));
    }
    out
}

/// Per-backend `WF-01`..`WF-65` human-path telemetry parsed from documentation and workspace tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackendWorkflowTelemetry {
    pub legacy_claimed: usize,
    pub egui_claimed: usize,
    pub legacy_backed: usize,
    pub egui_backed: usize,
}

fn parse_backend_wf_index(fn_name: &str) -> Option<(&'static str, u8)> {
    let rest = fn_name.strip_prefix("wf")?;
    let suffix = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let digits = rest.strip_suffix(suffix)?;
    if digits.is_empty() {
        return None;
    }
    let idx: u8 = digits.parse().ok()?;
    if !(1..=TOTAL_WORKFLOWS_U8).contains(&idx) {
        return None;
    }
    if suffix == "_human" || suffix.starts_with("_human_") || suffix == "_egui_human" || suffix.starts_with("_egui_human_") {
        Some(("egui", idx))
    } else {
        None
    }
}

/// Count distinct non-ignored `WF-01`..`WF-65` human-path tests per backend (`legacy`, `egui`).
#[must_use]
pub fn count_backed_backend_workflows(known_tests: &WorkspaceTests) -> (usize, usize) {
    let mut legacy_ids = BTreeSet::new();
    let mut egui_ids = BTreeSet::new();
    for qualified in &known_tests.qualified {
        let stem = qualified.split("::").next().unwrap_or("");
        if stem.starts_with("gui_wf_") {
            continue;
        }
        let fn_name = qualified.rsplit("::").next().unwrap_or(qualified);
        if let Some((backend, idx)) = parse_backend_wf_index(fn_name) {
            match backend {
                "legacy" => {
                    legacy_ids.insert(idx);
                }
                "egui" => {
                    egui_ids.insert(idx);
                }
                _ => {}
            }
        }
    }
    (legacy_ids.len(), egui_ids.len())
}

/// Validate per-backend workflow counts in `docs/evidence/user-interface.md` against backed workspace tests.
#[must_use]
pub fn check_backend_workflow_telemetry(
    path: &str,
    ui_parity_md: &str,
    known_tests: &WorkspaceTests,
) -> (BackendWorkflowTelemetry, Vec<EvidenceViolation>) {
    let (legacy_backed, egui_backed) = count_backed_backend_workflows(known_tests);
    let mut violations = Vec::new();
    let mut fence: Option<char> = None;
    let mut legacy_claims: Vec<(usize, usize)> = Vec::new();
    let mut egui_claims: Vec<(usize, usize)> = Vec::new();

    for (idx, line) in ui_parity_md.lines().enumerate() {
        let trimmed = line.trim();
        if update_fence(trimmed, &mut fence) || fence.is_some() {
            continue;
        }
        let is_checklist_item = trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]") || trimmed.starts_with("- [ ]");
        if !is_checklist_item || trimmed.to_ascii_lowercase().contains(AGENT_HARNESS_TAG) {
            continue;
        }
        for (num, den, backend) in parse_workflow_fractions(&trimmed.to_ascii_lowercase()) {
            if den != TOTAL_WORKFLOWS {
                continue;
            }
            match backend {
                Some("legacy") => legacy_claims.push((idx + 1, num)),
                Some("egui") => egui_claims.push((idx + 1, num)),
                _ => {}
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
        violations.push(EvidenceViolation {
            path: path.to_owned(),
            line: 1,
            reason: format!("missing `N/{TOTAL_WORKFLOWS} {backend}` workflow checklist count"),
        });
        return 0;
    };

    for &(line, num) in claims {
        if num != first_num {
            violations.push(EvidenceViolation {
                path: path.to_owned(),
                line,
                reason: format!(
                    "conflicting `{num}/{TOTAL_WORKFLOWS} {backend}` count (line {first_line} reports `{first_num}/{TOTAL_WORKFLOWS} {backend}`)"
                ),
            });
        }
        if num > TOTAL_WORKFLOWS || num > backed {
            violations.push(EvidenceViolation {
                path: path.to_owned(),
                line,
                reason: format!(
                    "claims `{num}/{TOTAL_WORKFLOWS} {backend}` human-path workflows, but only `{backed}/{TOTAL_WORKFLOWS}` backed non-ignored tests exist"
                ),
            });
        }
    }
    first_num
}

fn all_fractions_are_zero(text: &str) -> bool {
    let fractions = parse_workflow_fractions(text);
    !fractions.is_empty() && fractions.iter().all(|&(num, den, _)| num == 0 && den > 0)
}

fn has_invalid_backend_fraction(text: &str) -> bool {
    parse_workflow_fractions(text).into_iter().any(|(num, den, backend)| backend.is_some() && (den == 0 || num > den))
}

fn validate_human_evidence(path: &str, line: usize, text: &str, known_tests: Option<&WorkspaceTests>) -> Option<EvidenceViolation> {
    let tokens = extract_candidate_tokens(text);
    let mut harness_hits = Vec::new();
    let mut valid_tests = Vec::new();
    let mut unknown_tests = Vec::new();

    if has_invalid_backend_fraction(&text.to_ascii_lowercase()) {
        return Some(EvidenceViolation {
            path: path.to_owned(),
            line,
            reason: String::from("contains invalid per-backend workflow count fraction (expected `num/den legacy|egui` with 0 < den and num <= den)"),
        });
    }

    for tok in tokens {
        if is_human_driver_token(&tok) {
            continue;
        }
        if is_agent_harness_token(&tok) {
            harness_hits.push(tok);
        } else if is_test_identifier(&tok) {
            if known_tests.is_none_or(|set| set.contains_test(&tok)) {
                valid_tests.push(tok);
            } else {
                unknown_tests.push(tok);
            }
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

fn extract_candidate_tokens(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    let mut rest = text;
    while let Some((_, after_start)) = rest.split_once('`') {
        let Some((code, tail)) = after_start.split_once('`') else {
            break;
        };
        push_clean_tokens(code, &mut seen, &mut out);
        rest = tail;
    }
    push_clean_tokens(text, &mut seen, &mut out);
    out
}

fn push_clean_tokens(fragment: &str, seen: &mut BTreeSet<String>, out: &mut Vec<String>) {
    for raw in fragment.split(|c: char| c.is_whitespace() || c == ',') {
        let cleaned = raw.trim_matches(|c: char| matches!(c, '`' | '*' | '(' | ')' | '[' | ']' | ';' | '.' | ':' | '"' | '\''));
        if !cleaned.is_empty() && seen.insert(cleaned.to_owned()) {
            out.push(cleaned.to_owned());
        }
    }
}

fn is_agent_harness_token(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("gui_wf_")
        || lower.contains("::gui_wf_")
        || lower.contains("gui_only")
        || lower == "gui_actions"
        || lower.starts_with("gui_actions::")
        || lower == "control_io"
        || lower.starts_with("control_io::")
    {
        return true;
    }
    let last_seg = lower.rsplit("::").next().unwrap_or(&lower);
    if is_wf_harness_fn(last_seg) {
        return true;
    }
    AGENT_HARNESS_MARKERS.iter().any(|marker| {
        lower == *marker
            || lower.starts_with(&format!("{marker}::"))
            || lower.ends_with(&format!("::{marker}"))
            || last_seg == *marker
            || lower.contains(marker)
    })
}

fn is_human_driver_token(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    HUMAN_DRIVER_TOKENS.iter().any(|&marker| lower == marker || lower.ends_with(&format!("::{marker}")))
}

fn is_wf_harness_fn(seg: &str) -> bool {
    let Some(rest) = seg.strip_prefix("wf") else {
        return false;
    };
    let suffix = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    suffix.len() < rest.len()
        && suffix.starts_with('_')
        && !suffix.starts_with("_human_")
        && suffix != "_human"
        && !suffix.starts_with("_egui_human_")
        && suffix != "_egui_human"
}

fn is_test_identifier(tok: &str) -> bool {
    if tok.starts_with('.') || tok.starts_with('-') || tok.contains(['/', '\\', '+', '=', '>', '<', '?']) {
        return false;
    }
    if tok.contains("::") {
        let parts: Vec<&str> = tok.split("::").collect();
        if parts.len() < 2 {
            return false;
        }
        let all_idents =
            parts.iter().all(|seg| seg.starts_with(|c: char| c.is_ascii_lowercase()) && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let last = parts[parts.len() - 1];
        return all_idents && last.contains('_') && last.len() >= 4;
    }
    tok.len() >= 6
        && tok.contains('_')
        && tok.starts_with(|c: char| c.is_ascii_lowercase())
        && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn has_recorded_manual_run(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for prefix in [
        "manual:",
        "manual run:",
        "manual walkthrough:",
        "manual-verified:",
        "recorded manual:",
        "manual qa:",
        "computer-use:",
        "computer-use run:",
        "computer-use walkthrough:",
        "computer-use qa:",
        "recorded computer-use:",
        "computer use:",
        "computer use walkthrough:",
        "computer use qa:",
    ] {
        if let Some((before, after)) = lower.split_once(prefix)
            && !is_negated_prefix(before)
            && has_concrete_manual_detail(after)
        {
            return true;
        }
    }
    [
        "manual (",
        "manual run (",
        "manual walkthrough (",
        "computer-use (",
        "computer-use run (",
        "computer-use walkthrough (",
        "computer-use qa (",
        "computer use (",
        "computer use walkthrough (",
    ]
    .iter()
    .any(|marker| {
        lower.split_once(marker).is_some_and(|(before, after)| {
            !is_negated_prefix(before)
                && after.split_once(')').is_some_and(|(inside, _)| {
                    let clean = inside.trim();
                    clean.contains("202")
                        && !clean.contains("todo")
                        && !clean.contains("pending")
                        && !clean.contains("no ")
                        && !clean.contains("none")
                })
        })
    })
}

fn is_negated_prefix(before: &str) -> bool {
    let prev = before.split_whitespace().next_back().unwrap_or("");
    matches!(prev, "no" | "without")
}

fn has_concrete_manual_detail(after: &str) -> bool {
    let first = after
        .trim()
        .split(|c: char| c.is_whitespace() || c == ')' || c == ',' || c == ';')
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| matches!(c, '`' | '*' | '_' | '.' | ':' | ';' | ',' | '!' | '?'));
    !first.is_empty() && !matches!(first, "none" | "no" | "todo" | "n/a" | "pending" | "tbd" | "planned" | "missing" | "needed" | "untested")
}

/// Scan all `crates/ui-egui/tests/human_wf_*.rs` and `human_wf_*/**/*.rs` suites on disk
/// (excluding `human_wf_common`) for statically complete `WF-01`..`WF-65` `WorkflowRun` tests
/// and forbidden bypass/guard patterns (`R10.26`).
pub fn scan_human_wf_completion(root: &Path) -> Result<(BTreeSet<u8>, Vec<EvidenceViolation>), String> {
    let tests_dir = root.join("crates/ui-egui/tests");
    if !is_real_dir(&tests_dir) {
        return Ok((BTreeSet::new(), Vec::new()));
    }
    let mut paths = Vec::new();
    for path in crate::scan_util::read_dir_paths_sorted(&tests_dir)? {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if meta.is_file() && name.ends_with(".rs") && name.starts_with("human_wf_") {
            paths.push(path);
        } else if meta.is_dir() && name.starts_with("human_wf_") && name != "human_wf_common" {
            walk_file_paths(&path, &|_| false, &|p| has_ext_ci(p, &["rs"]), &mut paths)?;
        }
    }
    paths.sort();

    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        let rel = rel_slash_lossy(root, path);
        let src = fs::read_to_string(path).map_err(|e| format!("{rel}: {e}"))?;
        files.push((rel, src));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
    Ok(analyze_human_wf_sources(&refs))
}

/// Analyze `(rel_path, rust_source)` pairs for `WF-NN` `WorkflowRun` five-stage completeness and forbidden bypasses.
#[must_use]
pub fn analyze_human_wf_sources(sources: &[(&str, &str)]) -> (BTreeSet<u8>, Vec<EvidenceViolation>) {
    let mut complete = BTreeSet::new();
    let mut violations = Vec::new();
    for &(path, src) in sources {
        for func in extract_rust_functions(src) {
            if !func.is_test || parse_wf_fn_prefix(&func.name).is_some() || func.header_and_body_code.contains("WorkflowRun") {
                check_workflow_run_lint(path, &func, &mut violations);
            }
            if func.is_test
                && !func.is_ignored
                && let Some(idx) = parse_wf_fn_prefix(&func.name)
                && has_wf_start(&func.body_with_strings, idx)
                && has_finish(&func.body_code_only)
            {
                complete.insert(idx);
            }
        }
    }
    (complete, violations)
}

struct ScannedRustFn {
    is_test: bool,
    is_ignored: bool,
    name: String,
    line: usize,
    header_and_body_code: String,
    body_with_strings: String,
    body_code_only: String,
}

fn build_stripped_views(src: &str) -> (String, String) {
    let (code_only, with_strings) = crate::rust_lex::blank_comments_and_literals_both(src, false);
    (with_strings, code_only)
}

fn extract_rust_functions(src: &str) -> Vec<ScannedRustFn> {
    let (with_strings, code_only) = build_stripped_views(src);
    let bytes = code_only.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line_no = 1usize;
    let mut pending_test = false;
    let mut pending_ignored = false;

    while i < n {
        if i + 1 < n
            && bytes[i] == b'#'
            && bytes[i + 1] == b'['
            && let Some(code_rest) = code_only.get(i..)
            && let Some((attr, tail)) = split_balanced_attr(code_rest)
        {
            pending_test |= is_test_attribute(attr);
            pending_ignored |= is_ignore_attribute(attr);
            let consumed = code_rest.len() - tail.len();
            line_no += code_rest.get(..consumed).unwrap_or("").bytes().filter(|&b| b == b'\n').count();
            i += consumed;
            continue;
        }
        if is_word_at(&code_only, i, "fn") && code_only.get(..i).is_some_and(can_start_item_before) {
            let fn_start = i;
            let fn_line = line_no;
            let after_fn = code_only.get(i + 2..).unwrap_or("").trim_start();
            let name: String = after_fn.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if !name.is_empty()
                && let Some((body_open, body_close)) = find_fn_body_braces(bytes, i + 2)
            {
                out.push(ScannedRustFn {
                    is_test: pending_test,
                    is_ignored: pending_ignored,
                    name,
                    line: fn_line,
                    header_and_body_code: code_only.get(fn_start..=body_close).unwrap_or("").to_owned(),
                    body_with_strings: with_strings.get(body_open..=body_close).unwrap_or("").to_owned(),
                    body_code_only: code_only.get(body_open..=body_close).unwrap_or("").to_owned(),
                });
                line_no += code_only.get(i..=body_close).unwrap_or("").bytes().filter(|&b| b == b'\n').count();
                i = body_close + 1;
                pending_test = false;
                pending_ignored = false;
                continue;
            }
        }
        if matches!(bytes[i], b';' | b'{' | b'}') {
            pending_test = false;
            pending_ignored = false;
        }

        let step = code_only.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        if bytes[i] == b'\n' {
            line_no += 1;
        }
        i += step;
    }
    out
}

fn can_start_item_before(prefix: &str) -> bool {
    let trimmed = prefix.trim_end();
    if trimmed.is_empty() {
        return true;
    }
    if let Some(last) = trimmed.chars().next_back()
        && matches!(last, ';' | '{' | '}' | ']')
    {
        return true;
    }
    let last_word: String =
        trimmed.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
    matches!(last_word.as_str(), "pub" | "async" | "const" | "unsafe")
}

fn find_fn_body_braces(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut idx = start;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b';' if paren == 0 && bracket == 0 => return None,
            b'{' if paren == 0 && bracket == 0 => {
                let mut depth = 1usize;
                let mut end = idx + 1;
                while end < bytes.len() && depth > 0 {
                    if bytes[end] == b'{' {
                        depth += 1;
                    } else if bytes[end] == b'}' {
                        depth -= 1;
                    }
                    if depth == 0 {
                        return Some((idx, end));
                    }
                    end += 1;
                }
                return None;
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn is_word_at(text: &str, pos: usize, word: &str) -> bool {
    if !text.get(pos..).is_some_and(|s| s.starts_with(word)) {
        return false;
    }
    let left_ok = pos == 0 || !text.as_bytes()[pos - 1].is_ascii_alphanumeric() && text.as_bytes()[pos - 1] != b'_';
    let end = pos + word.len();
    let right_ok = end == text.len() || !text.as_bytes()[end].is_ascii_alphanumeric() && text.as_bytes()[end] != b'_';
    left_ok && right_ok
}

fn parse_wf_fn_prefix(fn_name: &str) -> Option<u8> {
    let rest = fn_name.strip_prefix("wf")?;
    let suffix = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let digits = rest.strip_suffix(suffix)?;
    if digits.is_empty() || !suffix.starts_with('_') {
        return None;
    }
    let idx: u8 = digits.parse().ok()?;
    (1..=TOTAL_WORKFLOWS_U8).contains(&idx).then_some(idx)
}

fn has_wf_start(body_with_strings: &str, idx: u8) -> bool {
    let compact: String = body_with_strings.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let p1 = format!("WorkflowRun::start(\"WF-{idx:02}\"");
    let p2 = format!("WorkflowRun::start_with_clipboard(\"WF-{idx:02}\"");
    compact.contains(&p1) || compact.contains(&p2)
}

fn has_finish(body_code_only: &str) -> bool {
    let compact: String = body_code_only.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if let Some(bound_var) = find_workflow_run_binding(body_code_only) {
        return compact.contains(&format!("{bound_var}.finish()")) || compact.contains(&format!("WorkflowRun::finish({bound_var})"));
    }
    has_chained_workflow_run_finish(&compact)
}

fn find_workflow_run_binding(code: &str) -> Option<String> {
    let bytes = code.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        if is_word_at(code, i, "let") {
            let mut j = i + 3;
            while j < n && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if is_word_at(code, j, "mut") {
                j += 3;
                while j < n && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
            }
            let ident_start = j;
            while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > ident_start {
                let ident = code.get(ident_start..j).unwrap_or("");
                while j < n && bytes[j] != b';' && bytes[j] != b'=' {
                    j += 1;
                }
                if j < n && bytes[j] == b'=' {
                    j += 1;
                    while j < n && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let rest: String = code.get(j..).unwrap_or("").chars().take(64).filter(|c| !c.is_ascii_whitespace()).collect();
                    if rest.starts_with("WorkflowRun::start") {
                        return Some(ident.to_owned());
                    }
                }
            }
        }
        i += code.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    }
    None
}

fn has_chained_workflow_run_finish(compact: &str) -> bool {
    let mut start = 0usize;
    while let Some(rel) = compact.get(start..).and_then(|s| s.find("WorkflowRun::start")) {
        let open_paren_rel = compact.get(start + rel..).and_then(|s| s.find('('));
        if let Some(open_rel) = open_paren_rel {
            let open_idx = start + rel + open_rel;
            let mut depth = 0usize;
            if let Some(paren_slice) = compact.get(open_idx..) {
                for (offset, b) in paren_slice.bytes().enumerate() {
                    match b {
                        b'(' => depth += 1,
                        b')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                let after = paren_slice.get(offset + 1..).unwrap_or("");
                                if after.starts_with(".finish()") || after.starts_with("?.finish()") {
                                    return true;
                                }
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        start += rel + 1;
    }
    false
}

fn contains_word_token(haystack: &str, needle: &str) -> bool {
    let mut start = 0usize;
    while let Some(rel) = haystack.get(start..).and_then(|s| s.find(needle)) {
        let pos = start + rel;
        if is_word_at(haystack, pos, needle) {
            return true;
        }
        start = pos + haystack.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    }
    false
}

fn has_guarded_find_by_widget_id(code: &str) -> bool {
    for (kw, allow_let) in [("if", true), ("while", true), ("match", false)] {
        for (i, _) in code.match_indices(kw) {
            if is_word_at(code, i, kw) && branch_header_contains_find(code, i + kw.len(), allow_let) {
                return true;
            }
        }
    }
    false
}

fn branch_header_contains_find(code: &str, after_kw: usize, allow_let: bool) -> bool {
    let bytes = code.as_bytes();
    let trimmed = code.get(after_kw..).unwrap_or("").trim_start();
    let start = code.len() - trimmed.len();
    let is_let_binding = allow_let && is_word_at(code, start, "let");
    let mut past_eq = !is_let_binding;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut pat_brace = 0usize;

    for (rel, ch) in trimmed.char_indices() {
        let j = start + rel;
        if trimmed.get(rel..).is_some_and(|s| s.starts_with("find_by_widget_id(")) {
            return true;
        }
        match ch {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            ';' if paren == 0 && bracket == 0 => return false,
            '=' if !past_eq && paren == 0 && bracket == 0 && pat_brace == 0 => {
                let prev = j.checked_sub(1).map(|k| bytes[k]);
                let next = bytes.get(j + 1).copied();
                if !matches!(prev, Some(b'!' | b'<' | b'>' | b'=')) && !matches!(next, Some(b'=' | b'>')) {
                    past_eq = true;
                }
            }
            '{' if !past_eq => pat_brace += 1,
            '}' if !past_eq => pat_brace = pat_brace.saturating_sub(1),
            '{' if past_eq && paren == 0 && bracket == 0 => return false,
            _ => {}
        }
    }
    false
}

fn check_workflow_run_lint(path: &str, func: &ScannedRustFn, violations: &mut Vec<EvidenceViolation>) {
    let code = &func.body_code_only;
    let compact: String = code.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    for token in ["execute_command", "app_mut", "state_mut", "dispatch_gui_action"] {
        if contains_word_token(code, token) {
            violations.push(EvidenceViolation {
                path: path.to_owned(),
                line: func.line,
                reason: format!("`WorkflowRun` test `{}` contains forbidden bypass `{token}`", func.name),
            });
        }
    }
    if compact.contains("Session::from_document") {
        violations.push(EvidenceViolation {
            path: path.to_owned(),
            line: func.line,
            reason: format!("`WorkflowRun` test `{}` contains forbidden seeded bypass `Session::from_document`", func.name),
        });
    }
    if has_guarded_find_by_widget_id(code) {
        violations.push(EvidenceViolation {
            path: path.to_owned(),
            line: func.line,
            reason: format!("`WorkflowRun` test `{}` guards a UI step on `if`/`if let` `find_by_widget_id(`", func.name),
        });
    }
}

fn parse_shard_wf_range(shard_cell: &str) -> Option<(u8, u8)> {
    let lower = shard_cell.to_ascii_lowercase();
    let mut nums = Vec::new();
    let bytes = lower.as_bytes();
    for (i, _) in lower.match_indices("wf-") {
        let mut end = i + 3;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > i + 3
            && let Some(val) = lower.get(i + 3..end).and_then(|s| s.parse::<u8>().ok())
            && (1..=TOTAL_WORKFLOWS_U8).contains(&val)
        {
            nums.push(val);
        }
    }
    if nums.len() >= 2 && nums[0] <= nums[1] {
        return Some((nums[0], nums[1]));
    }
    if nums.len() == 1 {
        return Some((nums[0], nums[0]));
    }
    for (pat, range) in [
        ("shard 1", (1, 6)),
        ("shard 2", (7, 15)),
        ("shard 3", (16, 23)),
        ("shard 4", (24, 28)),
        ("shard 5", (29, 36)),
        ("shard 6", (37, 43)),
        ("shard 7", (44, 49)),
        ("shard 8", (51, 58)),
        ("shard 9", (59, 65)),
        ("baseline", (1, 50)),
        ("extended", (51, 65)),
    ] {
        if lower.contains(pat) {
            return Some(range);
        }
    }
    None
}

fn parse_labeled_fraction(text: &str, label: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut out = Vec::new();
    let mut search_from = 0usize;

    while let Some(rel) = lower.get(search_from..).and_then(|s| s.find(label)) {
        let pos = search_from + rel;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let after_label = pos + label.len();
        search_from = after_label;
        if !left_ok {
            continue;
        }
        let mut cur = after_label;
        while cur < bytes.len() && (bytes[cur].is_ascii_whitespace() || matches!(bytes[cur], b'`' | b'*' | b'_' | b':')) {
            cur += 1;
        }
        let num_start = cur;
        while cur < bytes.len() && bytes[cur].is_ascii_digit() {
            cur += 1;
        }
        if cur == num_start {
            continue;
        }
        let Some(num) = lower.get(num_start..cur).and_then(|s| s.parse::<usize>().ok()) else {
            continue;
        };
        while cur < bytes.len() && bytes[cur].is_ascii_whitespace() {
            cur += 1;
        }
        if cur >= bytes.len() || bytes[cur] != b'/' {
            continue;
        }
        cur += 1;
        while cur < bytes.len() && bytes[cur].is_ascii_whitespace() {
            cur += 1;
        }
        let den_start = cur;
        while cur < bytes.len() && bytes[cur].is_ascii_digit() {
            cur += 1;
        }
        if cur > den_start
            && let Some(den) = lower.get(den_start..cur).and_then(|s| s.parse::<usize>().ok())
        {
            out.push((num, den));
        }
    }
    out
}

fn count_complete_in_range(complete_wfs: &BTreeSet<u8>, start: u8, end: u8) -> usize {
    (start..=end).filter(|i| complete_wfs.contains(i)).count()
}

fn validate_section62_completion_counts(
    path: &str,
    line: usize,
    shard_cell: &str,
    human_cell: &str,
    complete_wfs: &BTreeSet<u8>,
    violations: &mut Vec<EvidenceViolation>,
) {
    let complete_claims = parse_labeled_fraction(human_cell, "complete");
    let five_stage_claims = parse_labeled_fraction(human_cell, "five-stage");
    if complete_claims.is_empty() && five_stage_claims.is_empty() {
        return;
    }
    let Some((start, end)) = parse_shard_wf_range(shard_cell) else {
        push_ev(violations, path, line, format!("cannot resolve `WF` shard range for §6.2 row `{}`", shard_cell.trim()));
        return;
    };
    let expected_m = usize::from(end - start + 1);
    let actual = count_complete_in_range(complete_wfs, start, end);

    for (label, claims, hint) in
        [("complete", complete_claims, String::new()), ("five-stage", five_stage_claims, format!(" (use `complete {actual} / {expected_m}`)"))]
    {
        for (n, m) in claims {
            if m != expected_m || n != actual {
                push_ev(
                    violations,
                    path,
                    line,
                    format!(
                        "§6.2 row claims `{label} {n} / {m}` for `WF-{start:02}`..`WF-{end:02}`, but statically counted `{actual} / {expected_m}` five-stage complete tests{hint}"
                    ),
                );
            }
        }
    }
}

/// Fail if any checked `[x]` checklist item in `docs/evidence/user-interface.md` claims five-stage
/// completion beyond the statically counted `WorkflowRun` complete set (`R10.26`).
#[must_use]
pub fn check_ui_parity_five_stage_claims(path: &str, ui_parity_md: &str, complete_wfs: &BTreeSet<u8>) -> Vec<EvidenceViolation> {
    let mut violations = Vec::new();
    let mut fence: Option<char> = None;

    for (idx, line) in ui_parity_md.lines().enumerate() {
        let trimmed = line.trim();
        if update_fence(trimmed, &mut fence) || fence.is_some() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if !is_checked_markdown_item(trimmed) || lower.contains(AGENT_HARNESS_TAG) {
            continue;
        }
        let complete_fracs = parse_labeled_fraction(&lower, "complete");
        let five_stage_fracs = parse_labeled_fraction(&lower, "five-stage");
        if !(lower.contains("five-stage") || lower.contains("five stage") || !complete_fracs.is_empty()) {
            continue;
        }
        let (start, end) = if lower.contains("wf-01") && lower.contains("wf-50") {
            (1, 50)
        } else if lower.contains("wf-51") && lower.contains("wf-65") {
            (51, 65)
        } else {
            (1, TOTAL_WORKFLOWS_U8)
        };
        let expected_m = usize::from(end - start + 1);
        let actual = count_complete_in_range(complete_wfs, start, end);

        let mut explicit_claims: Vec<usize> = complete_fracs.into_iter().chain(five_stage_fracs).map(|(n, _)| n).collect();
        for (num, den, _) in parse_workflow_fractions(&lower) {
            if den == expected_m || den == TOTAL_WORKFLOWS {
                explicit_claims.push(num);
            }
        }
        let exceeds = if explicit_claims.is_empty() { actual < expected_m } else { explicit_claims.iter().any(|&n| n > actual) };
        if exceeds {
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

/// Scan every Markdown table in `md`, returning `(table_count, duplicate_keys)`.
#[must_use]
pub fn check_markdown_tables(path: &str, md: &str) -> (usize, Vec<DuplicateKey>) {
    let lines: Vec<&str> = md.lines().collect();
    let mut tables = 0usize;
    let mut duplicates = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut in_fence = false;
    let mut in_table = false;
    let mut idx = 0usize;

    while let Some(&raw) = lines.get(idx) {
        let trimmed = raw.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            in_table = false;
            seen.clear();
            idx += 1;
            continue;
        }
        if in_fence {
            idx += 1;
            continue;
        }

        if !in_table {
            if is_table_row(trimmed) && lines.get(idx + 1).is_some_and(|next| is_table_separator(next.trim())) {
                tables += 1;
                in_table = true;
                seen.clear();
                idx += 2; // Skip header + separator row.
                continue;
            }
            idx += 1;
            continue;
        }

        if is_table_row(trimmed) {
            if let Some(first_col) = parse_first_cell(trimmed) {
                let norm = normalize_key(&first_col);
                if !norm.is_empty() && !seen.insert(norm) {
                    duplicates.push(DuplicateKey { path: path.to_owned(), line: idx + 1, key: first_col });
                }
            }
            idx += 1;
        } else {
            in_table = false;
            seen.clear();
            idx += 1;
        }
    }
    (tables, duplicates)
}

fn is_table_row(trimmed: &str) -> bool {
    trimmed.starts_with('|') && trimmed.len() > 1
}

fn is_table_separator(trimmed: &str) -> bool {
    if !is_table_row(trimmed) {
        return false;
    }
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut saw_cell = false;
    for cell in inner.split('|') {
        let c = cell.trim();
        if c.is_empty() || !c.contains('-') || !c.bytes().all(|b| matches!(b, b'-' | b':' | b' ')) {
            return false;
        }
        saw_cell = true;
    }
    saw_cell
}

fn parse_first_cell(row: &str) -> Option<String> {
    let after_leading = row.trim().strip_prefix('|')?;
    let mut cell = String::new();
    let mut chars = after_leading.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                cell.push(next);
            }
        } else if ch == '|' {
            break;
        } else {
            cell.push(ch);
        }
    }
    Some(cell.trim().to_owned())
}

fn normalize_key(raw: &str) -> String {
    let mut s = raw.trim();
    loop {
        let prev = s;
        if let Some(inner) = s.strip_prefix("**").and_then(|v| v.strip_suffix("**")) {
            s = inner.trim();
        } else if let Some(inner) = s.strip_prefix("__").and_then(|v| v.strip_suffix("__")) {
            s = inner.trim();
        } else if let Some(inner) = s.strip_prefix('`').and_then(|v| v.strip_suffix('`')) {
            s = inner.trim();
        }
        if s == prev {
            break;
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}

/// Verify `docs/STATUS.md`, `docs/commands.md`, `AGENTS.md`, and `docs/control-protocol.md`
/// agree with live command, UI-command, and workspace-package counts (`R2-S13-11`).
#[must_use]
pub fn check_live_status_metrics(root: &Path, status_md: &str) -> Vec<EvidenceViolation> {
    check_live_status_metrics_with_ci(root, status_md, crate::workflows::is_ci_env())
}

fn check_live_status_metrics_with_ci(root: &Path, status_md: &str, in_ci: bool) -> Vec<EvidenceViolation> {
    let mut violations = Vec::new();
    #[cfg(feature = "docs-site")]
    let live_engine = Some(curvebook_engine::cmd::registry().len());
    #[cfg(not(feature = "docs-site"))]
    let live_engine: Option<usize> = {
        if in_ci {
            push_ev(
                &mut violations,
                STATUS_REL,
                1,
                String::from("xtask built without `--features docs-site` under CI; cannot verify live engine command count parity"),
            );
        } else if !cfg!(test) {
            eprintln!("status: warning: xtask built without `--features docs-site`; skipping live engine command count parity checks");
        }
        None
    };
    let _ = in_ci;
    let live_ui = match count_live_ui_commands(root) {
        Ok(n) => Some(n),
        Err(err) => {
            push_ev(&mut violations, "crates/ui-egui/src/menus/ui_commands.rs", 1, err);
            None
        }
    };
    let live_pkgs = crate::layers::TABLE.len();

    check_status_structural_headings(status_md, &mut violations);
    check_status_headline_table(status_md, live_engine, live_ui, live_pkgs, &mut violations);
    check_doc_count_parity(root, live_engine, live_ui, &mut violations);
    violations
}

fn count_live_ui_commands(root: &Path) -> Result<usize, String> {
    let rel = "crates/ui-egui/src/menus/ui_commands.rs";
    let src = fs::read_to_string(root.join(rel)).map_err(|e| format!("cannot read `{rel}` to count live UI commands: {e}"))?;
    count_ui_command_specs_in_source(&src).ok_or_else(|| format!("cannot parse `pub const UI_COMMANDS` array in `{rel}`"))
}

fn count_ui_command_specs_in_source(src: &str) -> Option<usize> {
    let (_with_strings, code_only) = build_stripped_views(src);
    let (_, after_decl) = code_only.split_once("pub const UI_COMMANDS:")?;
    let (_, after_eq) = after_decl.split_once('=')?;
    let open_rel = after_eq.find('[')?;
    let slice = after_eq.get(open_rel..)?;
    let mut depth = 0usize;
    let mut slice_end = None;
    for (idx, b) in slice.bytes().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    slice_end = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let body = slice.get(..slice_end?)?;
    Some(count_struct_literal_occurrences(body, "UiCommandSpec"))
}

fn count_struct_literal_occurrences(code: &str, type_name: &str) -> usize {
    let bytes = code.as_bytes();
    let mut count = 0usize;
    let mut search = 0usize;
    while let Some(rel) = code.get(search..).and_then(|s| s.find(type_name)) {
        let pos = search + rel;
        let end = pos + type_name.len();
        let left_ok = pos == 0 || !crate::rust_lex::is_ident_byte(bytes[pos - 1]);
        let right_ok = end == bytes.len() || !crate::rust_lex::is_ident_byte(bytes[end]);
        if left_ok && right_ok {
            let mut next = end;
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            if bytes.get(next) == Some(&b'{') {
                count += 1;
            }
        }
        search = end;
    }
    count
}

fn check_status_structural_headings(status_md: &str, violations: &mut Vec<EvidenceViolation>) {
    for heading in ["## Verified Capabilities & Headline Numbers", "## Known Limits"] {
        if !status_md.lines().any(|l| l.trim() == heading) {
            push_ev(violations, STATUS_REL, 1, format!("missing required section heading `{heading}`"));
        }
    }
}

fn check_status_headline_table(
    status_md: &str,
    live_engine: Option<usize>,
    live_ui: Option<usize>,
    live_pkgs: usize,
    violations: &mut Vec<EvidenceViolation>,
) {
    let engine_needle = live_engine.map(|n| n.to_string());
    let pkg_needle = format!("{live_pkgs}` packages");
    let pkg_plain = format!("{live_pkgs} packages");
    let (mut saw_engine, mut saw_ui, mut saw_platform) = (false, false, false);

    for (idx, raw) in status_md.lines().enumerate() {
        let trimmed = raw.trim();
        if !is_table_row(trimmed) || is_table_separator(trimmed) {
            continue;
        }
        let Some(col1) = parse_first_cell(trimmed) else {
            continue;
        };
        let norm = normalize_key(&col1).replace('`', "");
        let line_no = idx + 1;
        if norm == "engine command registry" {
            saw_engine = true;
            if let (Some(count), Some(needle)) = (live_engine, engine_needle.as_deref())
                && !cell_contains_number(trimmed, needle)
            {
                push_ev(violations, STATUS_REL, line_no, format!("Engine command registry row does not report live count `{count}`"));
            }
        } else if norm == "ui command catalog" {
            saw_ui = true;
            if let Some(ui_count) = live_ui
                && !cell_contains_number(trimmed, &ui_count.to_string())
            {
                push_ev(violations, STATUS_REL, line_no, format!("UI command catalog row does not report live count `{ui_count}`"));
            }
        } else if norm.starts_with("platform, r0 alignment") {
            saw_platform = true;
            if !trimmed.contains(&pkg_needle) && !trimmed.contains(&pkg_plain) {
                push_ev(violations, STATUS_REL, line_no, format!("Platform/R0 row does not report live `{live_pkgs}` workspace packages"));
            }
        }
    }

    for (present, label) in
        [(saw_engine, "Engine command registry"), (saw_ui, "UI command catalog"), (saw_platform, "Platform, `R0` alignment & Zero Telemetry")]
    {
        if !present {
            push_ev(violations, STATUS_REL, 1, format!("missing `{label}` row in Verified Capabilities table"));
        }
    }
}

fn cell_contains_number(row: &str, needle: &str) -> bool {
    let second_cell = row.split('|').nth(2).unwrap_or(row);
    let bytes = second_cell.as_bytes();
    let mut start = 0usize;
    while let Some(rel) = second_cell.get(start..).and_then(|s| s.find(needle)) {
        let pos = start + rel;
        let end = pos + needle.len();
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_digit();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_digit();
        if left_ok && right_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn check_doc_count_parity(root: &Path, live_engine: Option<usize>, live_ui: Option<usize>, violations: &mut Vec<EvidenceViolation>) {
    match fs::read_to_string(root.join("docs/commands.md")) {
        Ok(cmds_doc) => {
            if let Some(engine_count) = live_engine {
                let expected = format!("Registered commands: {engine_count}");
                if !cmds_doc.contains(&expected) {
                    push_ev(violations, "docs/commands.md", 1, format!("expected `{expected}` matching live registry"));
                }
            }
        }
        Err(err) => push_ev(violations, "docs/commands.md", 1, format!("cannot read `docs/commands.md`: {err}")),
    }
    match fs::read_to_string(root.join("AGENTS.md")) {
        Ok(agents_doc) => {
            if let Some(engine_count) = live_engine {
                let expected = format!("`{engine_count}` commands");
                if !agents_doc.contains(&expected) {
                    push_ev(violations, "AGENTS.md", 1, format!("expected `{expected}` matching live command registry"));
                }
            }
            if let Some(ui_count) = live_ui {
                let expected = format!("`{ui_count}` entries");
                if !agents_doc.contains(&expected) {
                    push_ev(violations, "AGENTS.md", 1, format!("expected `{expected}` matching live `UI_COMMANDS` count"));
                }
            }
        }
        Err(err) => push_ev(violations, "AGENTS.md", 1, format!("cannot read `AGENTS.md`: {err}")),
    }
    match fs::read_to_string(root.join("docs/control-protocol.md")) {
        Ok(proto_doc) => {
            if let Some(ui_count) = live_ui {
                let exact_ui = format!("`{ui_count}` synced `UI_COMMANDS`");
                if proto_doc.contains("`32` synced `UI_COMMANDS`")
                    || (!proto_doc.contains("all synced `UI_COMMANDS`") && !proto_doc.contains(&exact_ui))
                {
                    push_ev(
                        violations,
                        "docs/control-protocol.md",
                        1,
                        format!("expected `all synced `UI_COMMANDS`` or `{exact_ui}` (found stale count)"),
                    );
                }
            }
        }
        Err(err) => push_ev(violations, "docs/control-protocol.md", 1, format!("cannot read `docs/control-protocol.md`: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        DuplicateKey, WorkspaceTests, analyze_human_wf_sources, check_backend_workflow_telemetry, check_gui_workflows_human_path,
        check_gui_workflows_human_path_with_completion, check_live_status_metrics, check_markdown_tables, check_ui_parity_evidence,
        check_ui_parity_five_stage_claims, scan_tests_in_rust_source,
    };

    fn mk_tests(qualified_names: &[&str]) -> WorkspaceTests {
        let mut tests = WorkspaceTests::default();
        for &q in qualified_names {
            tests.qualified.insert(q.to_owned());
            if let Some(unqual) = q.rsplit("::").next() {
                tests.unqualified.insert(unqual.to_owned());
            }
        }
        tests
    }

    fn sec62_md(rows: &[&str]) -> String {
        let mut s = String::from(
            "### 6.2 Per-shard human-path status\n\n| Shard / workflows | Agent harness | Human path | Blockers | Gaps |\n|---|---|---|---|---|\n",
        );
        for r in rows {
            s.push_str(r);
            s.push('\n');
        }
        s
    }

    #[test]
    fn ui_parity_and_manual_run_evidence_rules() {
        let tests = mk_tests(&[
            "desktop_host_loop::file_drop_human_path",
            "gui_wf_foundations_and_data::wf01_one_sample_t_test",
            "gui_wf_50_cld::wf50_compact_letter_display_piepho_reactive_svg_pdf_rgba",
            "control_io::some_control_test",
        ]);
        let ok_md = "A checkbox `[x]` in prose is ignored.\n\
            - [x] File drop — verified by `desktop_host_loop::file_drop_human_path`.\n\
            - [x] Native Save — manual: 2026-11-16 macOS verified save dialog.\n\
            - [x] Native Save — computer-use: 2026-11-18 macOS desktop mouse+screen QA.\n\
            - [x] Native Print — computer-use walkthrough (2026-11-18 macOS + Safari).\n\
            - [x] *(agent harness)* Wizard suite (`gui_wf_foundations_and_data::wf01_t_test`, `--control`).\n\
            - [ ] Pending dialog work (`dispatch_gui_action` only).\n";
        assert!(check_ui_parity_evidence("docs/evidence/user-interface.md", ok_md, Some(&tests)).is_empty());

        let missing = check_ui_parity_evidence("docs/evidence/user-interface.md", "- [x] Interactive modal buttons work cleanly.\n", None);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].line == 1 && missing[0].reason.contains("missing passing human-path test"));

        let unknown = check_ui_parity_evidence(
            "docs/evidence/user-interface.md",
            "- [x] Modal dialog — verified by `ui_tests::invented_nonexistent_test`.\n",
            Some(&WorkspaceTests::default()),
        );
        assert_eq!(unknown.len(), 1);
        assert!(unknown[0].reason.contains("invented_nonexistent_test"));

        for harness_line in [
            "- [x] One-sample t-test dialog works (`gui_wf_foundations_and_data::wf01_one_sample_t_test`).\n",
            "- [x] CLD dialog works (`gui_wf_50_cld::wf50_compact_letter_display_piepho_reactive_svg_pdf_rgba`).\n",
            "- [x] Analyze wizard works (driven with `--control` and `--event-script`).\n",
            "- [x] Result chaining works via `dispatch_gui_action` and `ui.invoke_menu`.\n",
            "- [x] Pointer gestures work via `ui.pointer_gesture` and `ui_inspect`.\n",
            "- [x] Control server handles inputs (`control_io::some_control_test`).\n",
            "- [x] Replay works via `gui_only_workflows::eight_gui_actions_and_modal_guards_drive_live_ui_under_gui_only`.\n",
            "- [x] Save dialog — computer-use: 2026-11-18 driven via `--control` and `ui.click`.\n",
        ] {
            let v = check_ui_parity_evidence("docs/evidence/user-interface.md", harness_line, Some(&tests));
            assert_eq!(v.len(), 1, "{harness_line}: {v:?}");
            assert!(v[0].reason.contains("agent-harness"), "{harness_line}: {}", v[0].reason);
        }

        for negated in [
            "- [x] Native save — no manual run yet; manual: todo.\n",
            "- [x] Native export — manual: planned.\n",
            "- [x] Open dialog — manual walkthrough (pending 2026-11).\n",
            "- [x] Open dialog — no manual (2026-11-16) yet.\n",
            "- [x] Open dialog — manual (none recorded 2026).\n",
            "- [x] Native save — no computer-use run yet; computer-use: todo.\n",
            "- [x] Open dialog — computer-use walkthrough (pending 2026-11).\n",
            "- [x] Open dialog — without computer-use (2026-11-18) verification.\n",
        ] {
            let v = check_ui_parity_evidence("docs/evidence/user-interface.md", negated, None);
            assert_eq!(v.len(), 1, "{negated}: {v:?}");
        }
    }

    #[test]
    fn gui_workflows_section62_human_wf_and_backend_telemetry_rules() {
        let tests = mk_tests(&[
            "r102_human_shards::r102_shard1_pass",
            "gui_wf_foundations_and_data::wf01_column_t_test",
            "gui_wf_foundations_and_data::wf01_one_sample_t_test",
            "gui_wf_foundations_and_data::wf01_human_spoof",
            "other_suite::wf01_non_human_harness",
            "human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason",
            "human_wf_32_37::wf34_human_qpcr_ddct_calculated_variables_and_welch",
            "r15_1_driver_and_i18n::r15_1_smoke_human_path",
            "human_wf_01_06::wf01_egui_human_xy_dose_response",
        ]);

        let ok_table = sec62_md(&[
            "| Shard 1 | Pass | `1 / 6` (`r102_human_shards::r102_shard1_pass`) | `B1` | None |",
            "| Shard 2 | Pass | `0 / 9` | `B1` | Static dialog |",
            "| Shard 3 | Pass | Pass (manual: 2026-11-16 desktop QA) | None | None |",
            "| `WF-50` | Pass | `0 / 1` | `B1` | Inert inspector |",
            "| Shard 1 | Pass | `1 / 6` (`human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason`) | None | None |",
            "| Shard 6 | Pass | `1 / 6` (`wf34_human_qpcr_ddct_calculated_variables_and_welch`) | None | None |",
            "| Shard 1 | Pass | `0/6 legacy, 0/6 egui` | None | Pending |",
            "| Shard 2 | Pass | `0/9 legacy, 1/9 egui` (`human_wf_01_06::wf01_egui_human_xy_dose_response`) | None | Partial |",
            "| **Total** | **50 / 50** | **`0 / 50`** | | |",
        ]);
        assert!(check_gui_workflows_human_path("docs/gui-workflows.md", &ok_table, Some(&tests)).is_empty());

        let bad_table = sec62_md(&[
            "| Shard 1 | Pass | `2 / 6` | None | None |",
            "| Shard 2 | Pass | Pass (`gui_wf_foundations_and_data::wf01_column_t_test`) | None | None |",
            "| Shard 3 | Pass | `1 / 8` (`--event-script`) | None | None |",
            "| Shard 4 | Pass | Yes (`dispatch_gui_action`) | None | None |",
            "| Shard 1 | Pass | Pass (`wf01_non_human_harness`) | None | None |",
            "| Shard 1 | Pass | `0/6 legacy, 2/6 egui` (`egui_kittest`) | None | Unbacked |",
            "| Shard 2 | Pass | `55/50 legacy, 0/50 egui` (`human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason`) | None | Bad |",
            "| Shard 3 | Pass | `0/6 legacy, 1/6 egui` (`EguiHumanPathDriver` + `--control`) | None | Harness |",
        ]);
        let bad = check_gui_workflows_human_path("docs/gui-workflows.md", &bad_table, Some(&tests));
        let expected_reasons = [
            "missing passing human-path test",
            "agent-harness",
            "agent-harness",
            "agent-harness",
            "agent-harness",
            "missing passing human-path test",
            "invalid per-backend workflow count",
            "agent-harness",
        ];
        assert_eq!(bad.len(), expected_reasons.len(), "{bad:?}");
        for (v, needle) in bad.iter().zip(expected_reasons) {
            assert!(v.reason.contains(needle), "expected `{needle}`, got `{}`", v.reason);
        }

        let valid_parity = "- [x] WF-01 qualified (`human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason`).\n\
            - [x] WF-01 unqualified (`wf01_human_xy_dose_response_format_exclude_with_reason`).\n\
            - [x] EgUi slice — verified by `r15_1_driver_and_i18n::r15_1_smoke_human_path` using `EguiHumanPathDriver` and `egui_kittest`.\n\
            - [x] Workflows — `50/50 legacy, 1/50 egui`, verified by `human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason` and `human_wf_01_06::wf01_egui_human_xy_dose_response`.\n\
            - [ ] Pending egui port (`0/50 egui`, `50/50 legacy`).\n";
        assert!(check_ui_parity_evidence("docs/evidence/user-interface.md", valid_parity, Some(&tests)).is_empty());

        for (line, needle) in [
            ("- [x] Bad harness (`gui_wf_foundations_and_data::wf01_one_sample_t_test`).\n", "agent-harness"),
            ("- [x] Bad spoof (`gui_wf_foundations_and_data::wf01_human_spoof`).\n", "agent-harness"),
            ("- [x] Bad non-human wf (`other_suite::wf01_non_human_harness`).\n", "agent-harness"),
            ("- [x] Bad unqualified (`wf01_one_sample_t_test`).\n", "agent-harness"),
            ("- [x] Verified via `EguiHumanPathDriver` and `egui_kittest`.\n", "missing passing human-path test"),
        ] {
            let v = check_ui_parity_evidence("docs/evidence/user-interface.md", line, Some(&tests));
            assert_eq!(v.len(), 1, "{line}: {v:?}");
            assert!(v[0].reason.contains(needle), "{line}: {}", v[0].reason);
        }

        // R15.4 backend telemetry
        let all_65_names: Vec<String> = (1..=65).map(|i| format!("human_wf_demo::wf{i:02}_human_example")).collect();
        let refs: Vec<&str> = all_65_names.iter().map(String::as_str).collect();
        let all_65 = mk_tests(&refs);
        let (tel, v) = check_backend_workflow_telemetry(
            "docs/evidence/user-interface.md",
            "- [x] *(human path)* All 65 — `65/65 egui`\n- [x] *(human path — `egui`)* Shell (`65/65 egui`)\n",
            &all_65,
        );
        assert!(
            v.is_empty() && tel.legacy_claimed == 0 && tel.egui_claimed == 65 && tel.legacy_backed == 0 && tel.egui_backed == 65,
            "{tel:?} {v:?}"
        );
        let (_, bad_v) = check_backend_workflow_telemetry(
            "docs/evidence/user-interface.md",
            "- [x] *(human path)* All 65 — `65/65 legacy, 65/65 egui`\n- [ ] *(human path — `egui`)* Overhaul (`2/65 egui` vs `65/65 legacy`)\n",
            &all_65,
        );
        assert!(bad_v.iter().any(|x| x.reason.contains("only `0/65`")) && bad_v.iter().any(|x| x.reason.contains("conflicting")), "{bad_v:?}");
    }

    #[test]
    fn r10_26_and_fa_sec23_04_static_wf_completion_and_test_scanner() {
        let mut ignored = WorkspaceTests::default();
        let scan_src = r###"
            const CH: char = '\'';
            /* #[test] fn fake_block_comment_test() {} */
            const RAW: &str = r##"#[test] fn fake_raw_string_test() {}"##;
            #[test] #[ignore = "not ready"] fn skipped_human_test() {}
            #[test] #[ignore] fn single_line_skipped_test() {}
            #[ignore] fn non_test_ignored_helper() {}
            #[test] #[allow(dead_code)] fn one_line_active_test() {}
            #[test] fn active_human_test() {}
        "###;
        scan_tests_in_rust_source("desktop_host_loop", None, scan_src, &mut ignored);
        assert!(ignored.contains_test("desktop_host_loop::active_human_test") && ignored.contains_test("desktop_host_loop::one_line_active_test"));
        for hidden in ["skipped_human_test", "single_line_skipped_test", "fake_block_comment_test", "fake_raw_string_test"] {
            assert!(!ignored.unqualified.contains(hidden), "{hidden} should be excluded");
        }

        let valid_src = "#[test] fn wf01_human_xy_demo() -> Result<(), String> { let mut run = WorkflowRun::start(\"WF-01\"); run.finish() }\n\
            #[test] fn wf02_human_clip_demo() -> Result<(), String> { let mut run = WorkflowRun::start_with_clipboard(\"WF-02\", clip); run.finish() }\n";
        let (complete, lints) = analyze_human_wf_sources(&[("crates/ui-egui/tests/human_wf_01_06.rs", valid_src)]);
        assert!(lints.is_empty() && complete.into_iter().collect::<Vec<_>>() == vec![1, 2]);

        let seeded = "#[test] fn wf03_human_seeded() -> Result<(), String> { let s = Session::from_document(doc); app.execute_command(\"a\", ()); let mut run = WorkflowRun::start(\"WF-03\"); let _ = run.app_mut(); let _ = run.state_mut(); let _ = dispatch_gui_action(&mut app, \"x\", ()); run.finish() }";
        assert_eq!(analyze_human_wf_sources(&[("crates/ui-egui/tests/human_wf_01_06.rs", seeded)]).1.len(), 5);

        let guarded = "#[test] fn wf04_human_guarded_if() { let mut run = WorkflowRun::start(\"WF-04\"); if run.frame_meta().find_by_widget_id(\"a\").is_some() {} run.finish() }\n\
            #[test] fn wf05_human_guarded_if_let() { let mut run = WorkflowRun::start(\"WF-05\"); if let Some(_) = run.frame_meta().find_by_widget_id(\"b\") {} run.finish() }";
        let (_, guard_lints) = analyze_human_wf_sources(&[("crates/ui-egui/tests/human_wf_01_06.rs", guarded)]);
        assert_eq!(guard_lints.len(), 2);
        assert!(guard_lints.iter().all(|v| v.reason.contains("find_by_widget_id(")));

        let no_finish = "#[test] fn wf01_human_no_finish() -> Result<(), String> { let mut run = WorkflowRun::start(\"WF-01\"); Ok(()) }";
        let (c_no_fin, _) = analyze_human_wf_sources(&[("crates/ui-egui/tests/human_wf_01_06.rs", no_finish)]);
        assert!(c_no_fin.is_empty());

        let tests = mk_tests(&["human_wf_01_06::wf01_human_xy_demo"]);
        let one_complete = BTreeSet::from([1u8]);
        let ok_62 = sec62_md(&["| Shard 1 (`WF-01`–`WF-06`) | Pass | complete `1 / 6` (`human_wf_01_06::wf01_human_xy_demo`) | None | None |"]);
        assert!(check_gui_workflows_human_path_with_completion("docs/gui-workflows.md", &ok_62, Some(&tests), Some(&one_complete)).is_empty());
        let bad_62 = sec62_md(&["| Shard 1 (`WF-01`–`WF-06`) | Pass | complete `2 / 6` (`human_wf_01_06::wf01_human_xy_demo`) | None | None |"]);
        let mis = check_gui_workflows_human_path_with_completion("docs/gui-workflows.md", &bad_62, Some(&tests), Some(&c_no_fin));
        assert_eq!(mis.len(), 1);
        assert!(mis[0].reason.contains("statically counted `0 / 6`"));

        let bad_ui = check_ui_parity_five_stage_claims(
            "docs/evidence/user-interface.md",
            "- [x] *(human path)* Five-stage `WF-01`–`WF-50` complete (`human_wf_01_06::wf01_human_xy_demo`).\n",
            &one_complete,
        );
        assert_eq!(bad_ui.len(), 1);
        assert!(bad_ui[0].reason.contains("1/50"));

        // FA-SEC23-04: mod tests, helper bypass, decoy .finish(), while/match find_by_widget_id guards.
        let tricky = "fn sneaky_helper(app: &mut App) { let _ = app.app_mut(); }\n\
            mod tests {\n  type Run = WorkflowRun;\n\
              #[test] fn wf01_type_alias_bypass() { let mut run = Run::start(\"WF-01\"); let _ = dispatch_gui_action(&mut app, \"x\", ()); run.finish() }\n\
              #[test] fn wf02_decoy_finish_and_while_guard() { let mut run = WorkflowRun::start(\"WF-02\"); while run.frame_meta().find_by_widget_id(\"x\").is_some() {} let other = Dummy; other.finish(); }\n\
              #[test] fn wf03_match_guard() { let mut run = WorkflowRun::start(\"WF-03\"); match run.frame_meta().find_by_widget_id(\"y\") { Some(_) => {} None => {} } run.finish() }\n}";
        let (tricky_c, tricky_l) = analyze_human_wf_sources(&[("crates/ui-egui/tests/human_wf_01_06.rs", tricky)]);
        assert!(!tricky_c.contains(&2));
        assert!(tricky_l.iter().any(|v| v.reason.contains("app_mut")));
        assert!(tricky_l.iter().any(|v| v.reason.contains("dispatch_gui_action")));
        assert!(tricky_l.iter().filter(|v| v.reason.contains("find_by_widget_id(")).count() >= 2);
    }

    #[test]
    fn markdown_tables_and_live_status_metrics() {
        let (tables, dups) =
            check_markdown_tables("docs/ROADMAP.md", "| Item | Status |\n|---|---|\n| `R0.3` | todo |\n| `R0.4` | done |\n| `R0.3` | dup |\n");
        assert_eq!(tables, 1);
        assert_eq!(dups, vec![DuplicateKey { path: String::from("docs/ROADMAP.md"), line: 5, key: String::from("`R0.3`") }]);

        let (tables2, dups2) = check_markdown_tables(
            "docs/STATUS.md",
            "| Key | Val |\n|---|---|\n| Alpha | 1 |\n\n| Key | Val |\n|---|---|\n| **Alpha** | 2 |\n\n```markdown\n| Key | Val |\n|---|---|\n| Dup | a |\n| Dup | b |\n```\n",
        );
        assert!(tables2 == 2 && dups2.is_empty());

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let status_md = std::fs::read_to_string(root.join("docs/STATUS.md")).unwrap();
        assert!(check_live_status_metrics(root, &status_md).is_empty());
        let stale = status_md
            .replace("`106`** typed `CommandSpec`", "`95`** typed `CommandSpec`")
            .replace("`61`** typed `UiCommandSpec`", "`32`** typed `UiCommandSpec`");
        let stale_hits = check_live_status_metrics(root, &stale);
        #[cfg(feature = "docs-site")]
        assert!(stale_hits.iter().any(|v| v.reason.contains("Engine command registry")), "{stale_hits:?}");
        assert!(stale_hits.iter().any(|v| v.reason.contains("UI command catalog")), "{stale_hits:?}");

        // AUD-12-37: missing docs/catalog files fail closed, multiline `UiCommandSpec\n{` is counted,
        // and CI without `docs-site` fails closed.
        let multiline_ui = "pub const UI_COMMANDS: &[UiCommandSpec] = &[\n    UiCommandSpec\n    {\n        id: \"a\",\n    },\n    UiCommandSpec\t{\n        id: \"b\",\n    },\n];\n";
        assert_eq!(super::count_ui_command_specs_in_source(multiline_ui), Some(2));

        let empty_root = crate::scan_util::test_support::TestDir::new("status-missing");
        let missing_hits = super::check_live_status_metrics_with_ci(empty_root.path(), &status_md, true);
        assert!(missing_hits.iter().any(|v| v.path == "crates/ui-egui/src/menus/ui_commands.rs"), "{missing_hits:?}");
        assert!(missing_hits.iter().any(|v| v.path == "docs/commands.md"), "{missing_hits:?}");
        assert!(missing_hits.iter().any(|v| v.path == "AGENTS.md"), "{missing_hits:?}");
        assert!(missing_hits.iter().any(|v| v.path == "docs/control-protocol.md"), "{missing_hits:?}");
        #[cfg(not(feature = "docs-site"))]
        assert!(missing_hits.iter().any(|v| v.reason.contains("docs-site")), "{missing_hits:?}");
    }
}
