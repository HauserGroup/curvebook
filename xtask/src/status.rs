//! `cargo xtask status`: verify unique first-column keys in `docs/STATUS.md` and `docs/ROADMAP.md`,
//! and enforce test-evidence gates across `docs/evidence/user-interface.md` and `docs/gui-workflows.md` §6.2
//! (Pitfall #6: semantic agent-harness tests/flags never prove human GUI parity).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

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
    let ui_parity_text = fs::read_to_string(root.join(UI_EVIDENCE_REL)).map_err(|e| format!("{UI_EVIDENCE_REL}: {e}"))?;
    violations.extend(check_ui_parity_evidence(UI_EVIDENCE_REL, &ui_parity_text, Some(&workspace_tests)));
    let (telemetry, telemetry_violations) = check_backend_workflow_telemetry(UI_EVIDENCE_REL, &ui_parity_text, &workspace_tests);
    violations.extend(telemetry_violations);

    let workflows_text = fs::read_to_string(root.join(GUI_WORKFLOWS_REL)).map_err(|e| format!("{GUI_WORKFLOWS_REL}: {e}"))?;
    violations.extend(check_gui_workflows_human_path(GUI_WORKFLOWS_REL, &workflows_text, Some(&workspace_tests)));

    if duplicates.is_empty() && violations.is_empty() {
        println!(
            "status: verified {total_tables} Markdown table(s) across {STATUS_REL} and {ROADMAP_REL}, \
             live metrics, [x] evidence in {UI_EVIDENCE_REL}, and §6.2 Human path column in {GUI_WORKFLOWS_REL} \
             ({}/50 egui; backed human tests: {}/50 egui)",
            telemetry.egui_claimed, telemetry.egui_backed
        );
        return Ok(());
    }

    let mut errors = Vec::new();
    if !duplicates.is_empty() {
        let lines: Vec<String> = duplicates.iter().map(ToString::to_string).collect();
        errors.push(format!("{} duplicate table key(s) found:\n  {}", duplicates.len(), lines.join("\n  ")));
    }
    if !violations.is_empty() {
        let lines: Vec<String> = violations.iter().map(ToString::to_string).collect();
        errors.push(format!("{} missing/invalid [x] evidence violation(s):\n  {}", violations.len(), lines.join("\n  ")));
    }
    Err(errors.join("\n"))
}

/// Collect all non-ignored `#[test]` functions from `crates/` and `apps/` under `root`.
pub fn collect_workspace_tests(root: &Path) -> Result<WorkspaceTests, String> {
    let mut tests = WorkspaceTests::default();
    for dir_name in ["crates", "apps"] {
        let base = root.join(dir_name);
        if base.is_dir() {
            visit_rust_dir(&base, &mut tests)?;
        }
    }
    Ok(tests)
}

fn visit_rust_dir(dir: &Path, out: &mut WorkspaceTests) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry_res in entries {
        let entry = entry_res.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            let skip = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == "target" || n.starts_with('.'));
            if !skip {
                visit_rust_dir(&path, out)?;
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let src = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let parent =
                if matches!(stem, "mod" | "lib" | "main") { path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) } else { None };
            scan_tests_in_rust_source(stem, parent, &src, out);
        }
    }
    Ok(())
}

fn scan_tests_in_rust_source(stem: &str, parent_mod: Option<&str>, src: &str, out: &mut WorkspaceTests) {
    let mut pending_test = false;
    let mut pending_ignored = false;
    let mut attr_depth = 0isize;

    for line in src.lines() {
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
                    return Some((&input[..split], &input[split..]));
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
    let idx = trimmed.find("fn ")?;
    let prefix = trimmed[..idx].trim();
    let valid_prefix =
        prefix.is_empty() || prefix.split_whitespace().all(|kw| kw == "async" || kw == "unsafe" || kw == "const" || kw.starts_with("pub"));
    if !valid_prefix {
        return None;
    }
    let after_fn = trimmed[idx + 3..].trim_start();
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
#[must_use]
pub fn check_gui_workflows_human_path(path: &str, md: &str, known_tests: Option<&WorkspaceTests>) -> Vec<EvidenceViolation> {
    let mut violations = Vec::new();
    let mut state = Section62State::default();

    for (idx, line) in md.lines().enumerate() {
        let trimmed = line.trim();
        if update_fence(trimmed, &mut state.fence) || state.fence.is_some() {
            continue;
        }
        state.step(path, idx + 1, trimmed, known_tests, &mut violations);
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
    fn step(&mut self, path: &str, line_no: usize, trimmed: &str, known_tests: Option<&WorkspaceTests>, violations: &mut Vec<EvidenceViolation>) {
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
        self.visit_table_row(path, line_no, trimmed, known_tests, violations);
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
        if let Some(cell) = cells.get(col_idx)
            && human_path_requires_evidence(cell)
            && let Some(violation) = validate_human_evidence(path, line_no, cell, known_tests)
        {
            violations.push(violation);
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
    } else if let Some(dot_pos) = trimmed.find(['.', ')']) {
        let prefix = &trimmed[..dot_pos];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
            trimmed[dot_pos + 1..].strip_prefix(' ').map(str::trim_start)
        } else {
            None
        }
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

        let Ok(num) = text[left_start..left_end].parse::<usize>() else {
            continue;
        };
        let Ok(den) = text[right_start..right_end].parse::<usize>() else {
            continue;
        };
        let trailing = text[right_end..].trim_start().split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("");
        let backend = KNOWN_BACKENDS.iter().copied().find(|&b_name| trailing.eq_ignore_ascii_case(b_name));
        out.push((num, den, backend));
    }
    out
}

/// Per-backend `WF-01`..`WF-50` human-path telemetry parsed from documentation and workspace tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackendWorkflowTelemetry {
    pub legacy_claimed: usize,
    pub egui_claimed: usize,
    pub legacy_backed: usize,
    pub egui_backed: usize,
}

fn parse_backend_wf_index(fn_name: &str) -> Option<(&'static str, u8)> {
    let rest = fn_name.strip_prefix("wf")?;
    let digits_len = rest.chars().take_while(char::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    let idx: u8 = rest[..digits_len].parse().ok()?;
    if !(1..=50).contains(&idx) {
        return None;
    }
    let suffix = &rest[digits_len..];
    if suffix == "_human" || suffix.starts_with("_human_") || suffix == "_egui_human" || suffix.starts_with("_egui_human_") {
        Some(("egui", idx))
    } else {
        None
    }
}

/// Count distinct non-ignored `WF-01`..`WF-50` human-path tests per backend (`legacy`, `egui`).
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
            if den != 50 {
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
        violations.push(EvidenceViolation { path: path.to_owned(), line: 1, reason: format!("missing `N/50 {backend}` workflow checklist count") });
        return 0;
    };

    for &(line, num) in claims {
        if num != first_num {
            violations.push(EvidenceViolation {
                path: path.to_owned(),
                line,
                reason: format!("conflicting `{num}/50 {backend}` count (line {first_line} reports `{first_num}/50 {backend}`)"),
            });
        }
        if num > 50 || num > backed {
            violations.push(EvidenceViolation {
                path: path.to_owned(),
                line,
                reason: format!("claims `{num}/50 {backend}` human-path workflows, but only `{backed}/50` backed non-ignored tests exist"),
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
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        push_clean_tokens(&after_start[..end], &mut seen, &mut out);
        rest = &after_start[end + 1..];
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
    let digit_count = rest.chars().take_while(char::is_ascii_digit).count();
    let suffix = &rest[digit_count..];
    digit_count > 0
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
        if let Some(pos) = lower.find(prefix)
            && !is_negated_prefix(&lower[..pos])
            && has_concrete_manual_detail(&lower[pos + prefix.len()..])
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
        lower.find(marker).is_some_and(|pos| {
            !is_negated_prefix(&lower[..pos])
                && lower[pos + marker.len()..].split_once(')').is_some_and(|(inside, _)| {
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

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn ui_parity_accepts_cited_human_test_manual_run_and_agent_harness_tag() {
        let mut tests = WorkspaceTests::default();
        tests.qualified.insert(String::from("desktop_host_loop::file_drop_human_path"));
        tests.unqualified.insert(String::from("file_drop_human_path"));

        let md = "\
A checkbox `[x]` in prose is ignored.
- [x] File drop — verified by `desktop_host_loop::file_drop_human_path`.
- [x] Native Save — manual: 2026-11-16 macOS verified save dialog.
- [x] *(agent harness)* Wizard suite (`gui_wf_foundations_and_data::wf01_t_test`, `--control`).
- [ ] Pending dialog work (`dispatch_gui_action` only).
";
        let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", md, Some(&tests));
        assert!(violations.is_empty(), "unexpected violations: {violations:?}");
    }

    #[test]
    fn ui_parity_rejects_missing_evidence_on_checked_item() {
        let md = "- [x] Interactive modal buttons work cleanly.\n";
        let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", md, None);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].line, 1);
        assert!(violations[0].reason.contains("missing passing human-path test"));
    }

    #[test]
    fn ui_parity_rejects_unknown_test_name_when_workspace_indexed() {
        let tests = WorkspaceTests::default();
        let md = "- [x] Modal dialog — verified by `ui_tests::invented_nonexistent_test`.\n";
        let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", md, Some(&tests));
        assert_eq!(violations.len(), 1);
        assert!(violations[0].reason.contains("invented_nonexistent_test"));
    }

    #[test]
    fn ui_parity_rejects_agent_harness_only_evidence_for_human_item() {
        let mut tests = WorkspaceTests::default();
        tests.qualified.insert(String::from("gui_wf_foundations_and_data::wf01_one_sample_t_test"));
        tests.unqualified.insert(String::from("wf01_one_sample_t_test"));
        tests.qualified.insert(String::from("gui_wf_50_cld::wf50_compact_letter_display_piepho_reactive_svg_pdf_rgba"));
        tests.unqualified.insert(String::from("wf50_compact_letter_display_piepho_reactive_svg_pdf_rgba"));
        tests.qualified.insert(String::from("control_io::some_control_test"));
        tests.unqualified.insert(String::from("some_control_test"));

        let cases = [
            "- [x] One-sample t-test dialog works (`gui_wf_foundations_and_data::wf01_one_sample_t_test`).\n",
            "- [x] CLD dialog works (`gui_wf_50_cld::wf50_compact_letter_display_piepho_reactive_svg_pdf_rgba`).\n",
            "- [x] Analyze wizard works (driven with `--control` and `--event-script`).\n",
            "- [x] Result chaining works via `dispatch_gui_action` and `ui.invoke_menu`.\n",
            "- [x] Pointer gestures work via `ui.pointer_gesture` and `ui_inspect`.\n",
            "- [x] Control server handles inputs (`control_io::some_control_test`).\n",
            "- [x] Replay works via `gui_only_workflows::eight_gui_actions_and_modal_guards_drive_live_ui_under_gui_only`.\n",
        ];
        for md in cases {
            let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", md, Some(&tests));
            assert_eq!(violations.len(), 1, "expected harness rejection for `{md}`, got {violations:?}");
            assert!(violations[0].reason.contains("agent-harness"), "got `{}`", violations[0].reason);
        }
    }

    #[test]
    fn gui_workflows_human_path_accepts_zero_no_or_valid_evidence() {
        let mut tests = WorkspaceTests::default();
        tests.qualified.insert(String::from("r102_human_shards::r102_shard1_pass"));
        tests.unqualified.insert(String::from("r102_shard1_pass"));

        let md = "\
### 6.2 Per-shard human-path status

| Shard / workflows | Agent harness | Human path | Blockers | Gaps |
|---|---|---|---|---|
| Shard 1 | Pass | `1 / 6` (`r102_human_shards::r102_shard1_pass`) | `B1` | None |
| Shard 2 | Pass | `0 / 9` | `B1` | Static dialog |
| Shard 3 | Pass | Pass (manual: 2026-11-16 desktop QA) | None | None |
| `WF-50` | Pass | `0 / 1` | `B1` | Inert inspector |
| **Total** | **50 / 50** | **`0 / 50`** | | |
";
        let violations = check_gui_workflows_human_path("docs/gui-workflows.md", md, Some(&tests));
        assert!(violations.is_empty(), "unexpected violations: {violations:?}");
    }

    #[test]
    fn gui_workflows_human_path_rejects_unbacked_pass_and_harness_only_claims() {
        let mut tests = WorkspaceTests::default();
        tests.qualified.insert(String::from("gui_wf_foundations_and_data::wf01_column_t_test"));

        let md = "\
### 6.2 Per-shard human-path status

| Shard / workflows | Agent harness | Human path | Blockers | Gaps |
|---|---|---|---|---|
| Shard 1 | Pass | `2 / 6` | None | None |
| Shard 2 | Pass | Pass (`gui_wf_foundations_and_data::wf01_column_t_test`) | None | None |
| Shard 3 | Pass | `1 / 8` (`--event-script`) | None | None |
| Shard 4 | Pass | Yes (`dispatch_gui_action`) | None | None |
";
        let violations = check_gui_workflows_human_path("docs/gui-workflows.md", md, Some(&tests));
        assert_eq!(violations.len(), 4, "expected 4 violations, got {violations:?}");
        assert!(violations[0].reason.contains("missing passing human-path test"));
        assert!(violations[1].reason.contains("agent-harness"));
        assert!(violations[2].reason.contains("agent-harness"));
        assert!(violations[3].reason.contains("agent-harness"));
    }

    #[test]
    fn manual_run_rejects_negated_or_todo_mentions() {
        let cases = [
            "- [x] Native save — no manual run yet; manual: todo.\n",
            "- [x] Native export — manual: planned.\n",
            "- [x] Open dialog — manual walkthrough (pending 2026-11).\n",
            "- [x] Open dialog — no manual (2026-11-16) yet.\n",
            "- [x] Open dialog — manual (none recorded 2026).\n",
            "- [x] Native save — no computer-use run yet; computer-use: todo.\n",
            "- [x] Open dialog — computer-use walkthrough (pending 2026-11).\n",
            "- [x] Open dialog — without computer-use (2026-11-18) verification.\n",
        ];
        for md in cases {
            let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", md, None);
            assert_eq!(violations.len(), 1, "expected rejection for `{md}`, got {violations:?}");
        }
    }

    #[test]
    fn computer_use_walkthrough_accepts_mouse_screen_run_and_rejects_harness_mix() {
        let valid_md = "\
- [x] Native Save — computer-use: 2026-11-18 macOS desktop mouse+screen QA.
- [x] Native Print — computer-use walkthrough (2026-11-18 macOS + Safari).
";
        let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", valid_md, None);
        assert!(violations.is_empty(), "unexpected violations: {violations:?}");

        let harness_mixed = "- [x] Save dialog — computer-use: 2026-11-18 driven via `--control` and `ui.click`.\n";
        let bad = check_ui_parity_evidence("docs/evidence/user-interface.md", harness_mixed, None);
        assert_eq!(bad.len(), 1, "expected agent-harness rejection, got {bad:?}");
        assert!(bad[0].reason.contains("agent-harness"), "got `{}`", bad[0].reason);
    }

    #[test]
    fn ignored_test_is_not_indexed_as_passing_evidence() {
        let src = "\
#[test]
#[ignore = \"not ready\"]
fn skipped_human_test() {}

#[test] #[ignore] fn single_line_skipped_test() {}

#[ignore]
fn non_test_ignored_helper() {}

#[test] #[allow(dead_code)] fn one_line_active_test() {}

#[test]
fn active_human_test() {}
";
        let mut tests = WorkspaceTests::default();
        scan_tests_in_rust_source("desktop_host_loop", None, src, &mut tests);
        assert!(tests.contains_test("desktop_host_loop::active_human_test"));
        assert!(tests.contains_test("desktop_host_loop::one_line_active_test"));
        assert!(!tests.contains_test("desktop_host_loop::skipped_human_test"));
        assert!(!tests.contains_test("desktop_host_loop::single_line_skipped_test"));
    }

    #[test]
    fn r10_2_accepts_human_wf_and_rejects_gui_wf_and_non_human_wf_fns() {
        let mut tests = WorkspaceTests::default();
        for (qual, unqual) in [
            ("human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason", "wf01_human_xy_dose_response_format_exclude_with_reason"),
            ("human_wf_32_37::wf34_human_qpcr_ddct_calculated_variables_and_welch", "wf34_human_qpcr_ddct_calculated_variables_and_welch"),
            ("gui_wf_foundations_and_data::wf01_one_sample_t_test", "wf01_one_sample_t_test"),
            ("gui_wf_foundations_and_data::wf01_human_spoof", "wf01_human_spoof"),
            ("other_suite::wf01_non_human_harness", "wf01_non_human_harness"),
        ] {
            tests.qualified.insert(String::from(qual));
            tests.unqualified.insert(String::from(unqual));
        }

        let valid_md = "\
- [x] WF-01 qualified (`human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason`).
- [x] WF-01 unqualified (`wf01_human_xy_dose_response_format_exclude_with_reason`).
";
        let valid_parity = check_ui_parity_evidence("docs/evidence/user-interface.md", valid_md, Some(&tests));
        assert!(valid_parity.is_empty(), "unexpected violations: {valid_parity:?}");

        let valid_wf_table = "\
### 6.2 Per-shard human-path status

| Shard / workflows | Agent harness | Human path | Blockers | Gaps |
|---|---|---|---|---|
| Shard 1 | Pass | `1 / 6` (`human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason`) | None | None |
| Shard 6 | Pass | `1 / 6` (`wf34_human_qpcr_ddct_calculated_variables_and_welch`) | None | None |
";
        let valid_wf = check_gui_workflows_human_path("docs/gui-workflows.md", valid_wf_table, Some(&tests));
        assert!(valid_wf.is_empty(), "unexpected violations: {valid_wf:?}");

        for bad_line in [
            "- [x] Bad harness (`gui_wf_foundations_and_data::wf01_one_sample_t_test`).\n",
            "- [x] Bad spoof (`gui_wf_foundations_and_data::wf01_human_spoof`).\n",
            "- [x] Bad non-human wf (`other_suite::wf01_non_human_harness`).\n",
            "- [x] Bad unqualified (`wf01_one_sample_t_test`).\n",
        ] {
            let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", bad_line, Some(&tests));
            assert_eq!(violations.len(), 1, "expected rejection for `{bad_line}`, got {violations:?}");
            assert!(violations[0].reason.contains("agent-harness"), "got `{}`", violations[0].reason);
        }

        let bad_wf_table = "\
### 6.2 Per-shard human-path status

| Shard / workflows | Agent harness | Human path | Blockers | Gaps |
|---|---|---|---|---|
| Shard 1 | Pass | Pass (`gui_wf_foundations_and_data::wf01_one_sample_t_test`) | None | None |
| Shard 2 | Pass | Pass (`wf01_non_human_harness`) | None | None |
";
        let bad_wf = check_gui_workflows_human_path("docs/gui-workflows.md", bad_wf_table, Some(&tests));
        assert_eq!(bad_wf.len(), 2, "expected 2 violations, got {bad_wf:?}");
        assert!(bad_wf.iter().all(|v| v.reason.contains("agent-harness")), "got {bad_wf:?}");
    }

    #[test]
    fn r15_1_accepts_egui_driver_with_real_test_and_enforces_per_backend_counts() {
        let mut tests = WorkspaceTests::default();
        tests.qualified.insert(String::from("r15_1_driver_and_i18n::r15_1_smoke_human_path"));
        tests.unqualified.insert(String::from("r15_1_smoke_human_path"));
        tests.qualified.insert(String::from("human_wf_01_06::wf01_egui_human_xy_dose_response"));
        tests.unqualified.insert(String::from("wf01_egui_human_xy_dose_response"));
        tests.qualified.insert(String::from("human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason"));
        tests.unqualified.insert(String::from("wf01_human_xy_dose_response_format_exclude_with_reason"));

        let valid_parity = "\
- [x] EgUi slice — verified by `r15_1_driver_and_i18n::r15_1_smoke_human_path` using `EguiHumanPathDriver` and `egui_kittest`.
- [x] Workflows — `50/50 legacy, 1/50 egui`, verified by `human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason` and `human_wf_01_06::wf01_egui_human_xy_dose_response`.
- [ ] Pending egui port (`0/50 egui`, `50/50 legacy`).
";
        let violations = check_ui_parity_evidence("docs/evidence/user-interface.md", valid_parity, Some(&tests));
        assert!(violations.is_empty(), "unexpected violations: {violations:?}");

        // Mentioning `EguiHumanPathDriver` or `egui_kittest` alone without a real passing test or manual run still fails (Pitfall #6).
        let unbacked_driver = "- [x] Verified via `EguiHumanPathDriver` and `egui_kittest`.\n";
        let unbacked_hits = check_ui_parity_evidence("docs/evidence/user-interface.md", unbacked_driver, Some(&tests));
        assert_eq!(unbacked_hits.len(), 1, "{unbacked_hits:?}");
        assert!(unbacked_hits[0].reason.contains("missing passing human-path test"));

        // Per-backend counts in §6.2 accept both `0/6 legacy, 0/6 egui`, mixed backed counts, and reject unbacked non-zero `egui` or malformed fractions.
        let valid_wf_table = "\
### 6.2 Per-shard human-path status

| Shard / workflows | Agent harness | Human path | Blockers | Gaps |
|---|---|---|---|---|
| Shard 1 | Pass | `0/6 legacy, 0/6 egui` | None | Pending |
| Shard 2 | Pass | `0/9 legacy, 1/9 egui` (`human_wf_01_06::wf01_egui_human_xy_dose_response`) | None | Partial |
";
        let ok_wf = check_gui_workflows_human_path("docs/gui-workflows.md", valid_wf_table, Some(&tests));
        assert!(ok_wf.is_empty(), "unexpected dual-backend violations: {ok_wf:?}");

        let bad_wf_table = "\
### 6.2 Per-shard human-path status

| Shard / workflows | Agent harness | Human path | Blockers | Gaps |
|---|---|---|---|---|
| Shard 1 | Pass | `0/6 legacy, 2/6 egui` (`egui_kittest`) | None | Unbacked |
| Shard 2 | Pass | `55/50 legacy, 0/50 egui` (`human_wf_01_06::wf01_human_xy_dose_response_format_exclude_with_reason`) | None | Out-of-bounds |
| Shard 3 | Pass | `0/6 legacy, 1/6 egui` (`EguiHumanPathDriver` + `--control`) | None | Harness |
";
        let bad_wf = check_gui_workflows_human_path("docs/gui-workflows.md", bad_wf_table, Some(&tests));
        assert_eq!(bad_wf.len(), 3, "expected 3 dual-backend violations, got {bad_wf:?}");
        assert!(bad_wf[0].reason.contains("missing passing human-path test"));
        assert!(bad_wf[1].reason.contains("invalid per-backend workflow count"));
        assert!(bad_wf[2].reason.contains("agent-harness"));
    }

    #[test]
    fn r15_4_backend_workflow_telemetry_enforces_consistency_and_backed_counts() {
        let mut tests = WorkspaceTests::default();
        for idx in 1..=50 {
            let egui_name = format!("wf{idx:02}_human_example");
            tests.unqualified.insert(egui_name.clone());
            tests.qualified.insert(format!("human_wf_demo::{egui_name}"));
        }

        let valid_md = "\
- [x] *(human path)* All 50 reference workflows — `50/50 egui`
- [x] *(human path — `egui` backend, `R15.5`)* Native `egui` desktop shell (`50/50 egui`)
";
        let (telemetry, violations) = super::check_backend_workflow_telemetry("docs/evidence/user-interface.md", valid_md, &tests);
        assert!(violations.is_empty(), "unexpected violations: {violations:?}");
        assert_eq!(telemetry.legacy_claimed, 0);
        assert_eq!(telemetry.egui_claimed, 50);
        assert_eq!(telemetry.legacy_backed, 0);
        assert_eq!(telemetry.egui_backed, 50);

        let stale_legacy_md = "\
- [x] *(human path)* All 50 reference workflows — `50/50 legacy, 50/50 egui`
- [ ] *(human path — `egui` backend)* Overhaul (`2/50 egui` vs `50/50 legacy`)
";
        let (_bad_tel, bad_violations) = super::check_backend_workflow_telemetry("docs/evidence/user-interface.md", stale_legacy_md, &tests);
        assert!(bad_violations.iter().any(|v| v.reason.contains("only `0/50`")), "{bad_violations:?}");
        assert!(bad_violations.iter().any(|v| v.reason.contains("conflicting")), "{bad_violations:?}");
    }
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
    let mut violations = Vec::new();
    let live_engine = curvebook_engine::cmd::registry().len();
    let live_ui = count_live_ui_commands(root);
    let live_pkgs = crate::layers::TABLE.len();

    check_status_structural_headings(status_md, &mut violations);
    check_status_headline_table(status_md, live_engine, live_ui, live_pkgs, &mut violations);
    check_doc_count_parity(root, live_engine, live_ui, &mut violations);
    violations
}

fn count_live_ui_commands(root: &Path) -> Option<usize> {
    let rel = "crates/ui-egui/src/menus/ui_commands.rs";
    let src = fs::read_to_string(root.join(rel)).ok()?;
    let slice_start = src.find("pub const UI_COMMANDS:")?;
    let slice_tail = &src[slice_start..];
    let slice_end = slice_tail.find("];")?;
    Some(slice_tail[..slice_end].matches("UiCommandSpec {").count())
}

fn check_status_structural_headings(status_md: &str, violations: &mut Vec<EvidenceViolation>) {
    for heading in ["## Verified Capabilities & Headline Numbers", "## Known Limits"] {
        if !status_md.lines().any(|l| l.trim() == heading) {
            violations.push(EvidenceViolation {
                path: String::from(STATUS_REL),
                line: 1,
                reason: format!("missing required section heading `{heading}`"),
            });
        }
    }
}

fn check_status_headline_table(
    status_md: &str,
    live_engine: usize,
    live_ui: Option<usize>,
    live_pkgs: usize,
    violations: &mut Vec<EvidenceViolation>,
) {
    let engine_needle = live_engine.to_string();
    let pkg_needle = format!("{live_pkgs}` packages");
    let pkg_plain = format!("{live_pkgs} packages");
    let mut saw_engine_row = false;
    let mut saw_ui_row = false;
    let mut saw_platform_row = false;

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
            saw_engine_row = true;
            if !cell_contains_number(trimmed, &engine_needle) {
                violations.push(EvidenceViolation {
                    path: String::from(STATUS_REL),
                    line: line_no,
                    reason: format!("Engine command registry row does not report live count `{live_engine}`"),
                });
            }
        } else if norm == "ui command catalog" {
            saw_ui_row = true;
            if let Some(ui_count) = live_ui
                && !cell_contains_number(trimmed, &ui_count.to_string())
            {
                violations.push(EvidenceViolation {
                    path: String::from(STATUS_REL),
                    line: line_no,
                    reason: format!("UI command catalog row does not report live count `{ui_count}`"),
                });
            }
        } else if norm.starts_with("platform, r0 alignment") {
            saw_platform_row = true;
            if !trimmed.contains(&pkg_needle) && !trimmed.contains(&pkg_plain) {
                violations.push(EvidenceViolation {
                    path: String::from(STATUS_REL),
                    line: line_no,
                    reason: format!("Platform/R0 row does not report live `{live_pkgs}` workspace packages"),
                });
            }
        }
    }

    for (present, label) in [
        (saw_engine_row, "Engine command registry"),
        (saw_ui_row, "UI command catalog"),
        (saw_platform_row, "Platform, `R0` alignment & Zero Telemetry"),
    ] {
        if !present {
            violations.push(EvidenceViolation {
                path: String::from(STATUS_REL),
                line: 1,
                reason: format!("missing `{label}` row in Verified Capabilities table"),
            });
        }
    }
}

fn cell_contains_number(row: &str, needle: &str) -> bool {
    let second_cell = row.split('|').nth(2).unwrap_or(row);
    let bytes = second_cell.as_bytes();
    let mut start = 0usize;
    while let Some(rel) = second_cell[start..].find(needle) {
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

fn check_doc_count_parity(root: &Path, live_engine: usize, live_ui: Option<usize>, violations: &mut Vec<EvidenceViolation>) {
    let cmds_rel = "docs/commands.md";
    if let Ok(cmds_doc) = fs::read_to_string(root.join(cmds_rel)) {
        let expected = format!("Registered commands: {live_engine}");
        if !cmds_doc.contains(&expected) {
            violations.push(EvidenceViolation {
                path: String::from(cmds_rel),
                line: 1,
                reason: format!("expected `{expected}` matching live registry"),
            });
        }
    }

    let agents_rel = "AGENTS.md";
    if let Ok(agents_doc) = fs::read_to_string(root.join(agents_rel)) {
        let expected_engine = format!("`{live_engine}` commands");
        if !agents_doc.contains(&expected_engine) {
            violations.push(EvidenceViolation {
                path: String::from(agents_rel),
                line: 1,
                reason: format!("expected `{expected_engine}` matching live command registry"),
            });
        }
        if let Some(ui_count) = live_ui {
            let expected_ui = format!("`{ui_count}` entries");
            if !agents_doc.contains(&expected_ui) {
                violations.push(EvidenceViolation {
                    path: String::from(agents_rel),
                    line: 1,
                    reason: format!("expected `{expected_ui}` matching live `UI_COMMANDS` count"),
                });
            }
        }
    }

    let proto_rel = "docs/control-protocol.md";
    if let Ok(proto_doc) = fs::read_to_string(root.join(proto_rel))
        && let Some(ui_count) = live_ui
    {
        let exact_ui = format!("`{ui_count}` synced `UI_COMMANDS`");
        if proto_doc.contains("`32` synced `UI_COMMANDS`") || (!proto_doc.contains("all synced `UI_COMMANDS`") && !proto_doc.contains(&exact_ui)) {
            violations.push(EvidenceViolation {
                path: String::from(proto_rel),
                line: 1,
                reason: format!("expected `all synced `UI_COMMANDS`` or `{exact_ui}` (found stale count)"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DuplicateKey, check_markdown_tables};

    #[test]
    fn fails_on_duplicate_first_column_key_within_single_table() {
        let fixture = "\
| Item | Status |
|---|---|
| `R0.3` | todo |
| `R0.4` | done |
| `R0.3` | conflict-duplicate |
";
        let (tables, dups) = check_markdown_tables("docs/ROADMAP.md", fixture);
        assert_eq!(tables, 1);
        assert_eq!(dups, vec![DuplicateKey { path: String::from("docs/ROADMAP.md"), line: 5, key: String::from("`R0.3`") }]);
    }

    #[test]
    fn allows_same_key_in_separate_tables_and_ignores_fenced_tables() {
        let fixture = "\
| Key | Val |
|---|---|
| Alpha | 1 |

| Key | Val |
|---|---|
| **Alpha** | 2 |

```markdown
| Key | Val |
|---|---|
| Dup | a |
| Dup | b |
```
";
        let (tables, dups) = check_markdown_tables("docs/STATUS.md", fixture);
        assert_eq!(tables, 2);
        assert!(dups.is_empty(), "{dups:?}");
    }

    #[test]
    fn r2_s13_11_live_status_metrics_passes_real_repo_and_catches_stale_counts() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let status_md = std::fs::read_to_string(root.join("docs/STATUS.md")).unwrap();
        let live_hits = super::check_live_status_metrics(root, &status_md);
        assert!(live_hits.is_empty(), "real workspace live metric drift: {live_hits:?}");

        let stale_status = status_md
            .replace("`105`** typed `CommandSpec`", "`95`** typed `CommandSpec`")
            .replace("`50`** typed `UiCommandSpec`", "`32`** typed `UiCommandSpec`");
        let stale_hits = super::check_live_status_metrics(root, &stale_status);
        assert!(stale_hits.iter().any(|v| v.reason.contains("Engine command registry")), "{stale_hits:?}");
        assert!(stale_hits.iter().any(|v| v.reason.contains("UI command catalog")), "{stale_hits:?}");
    }
}
