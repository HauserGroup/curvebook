//! `cargo xtask status`: verify unique first-column keys in `docs/STATUS.md` and `docs/ROADMAP.md`,
//! and enforce test-evidence gates across `docs/ui-parity.md` and `docs/gui-workflows.md` §6.2
//! (Pitfall #6: semantic agent-harness tests/flags never prove human GUI parity).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const STATUS_REL: &str = "docs/STATUS.md";
const ROADMAP_REL: &str = "docs/ROADMAP.md";
const UI_PARITY_REL: &str = "docs/ui-parity.md";
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
/// human-path test or recorded manual verification evidence.
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
/// and enforce `[x]` / human-path evidence across `docs/ui-parity.md` and `docs/gui-workflows.md`.
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

    let workspace_tests = collect_workspace_tests(root)?;
    let ui_parity_text = fs::read_to_string(root.join(UI_PARITY_REL)).map_err(|e| format!("{UI_PARITY_REL}: {e}"))?;
    let mut violations = check_ui_parity_evidence(UI_PARITY_REL, &ui_parity_text, Some(&workspace_tests));

    let workflows_text = fs::read_to_string(root.join(GUI_WORKFLOWS_REL)).map_err(|e| format!("{GUI_WORKFLOWS_REL}: {e}"))?;
    violations.extend(check_gui_workflows_human_path(GUI_WORKFLOWS_REL, &workflows_text, Some(&workspace_tests)));

    if duplicates.is_empty() && violations.is_empty() {
        println!(
            "status: verified {total_tables} Markdown table(s) across {STATUS_REL} and {ROADMAP_REL}, \
             [x] evidence in {UI_PARITY_REL}, and §6.2 Human path column in {GUI_WORKFLOWS_REL}"
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

/// Enforce test or manual-run evidence on every `[x]` item/row in `docs/ui-parity.md`
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
    !is_zero_fraction(&lower)
}

fn is_zero_fraction(text: &str) -> bool {
    let Some((left, right)) = text.split_once('/') else {
        return false;
    };
    let num = left.trim().trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let den_tok = right.split_whitespace().next().unwrap_or("");
    let den = den_tok.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    num == "0" && !den.is_empty() && den.chars().all(|c| c.is_ascii_digit()) && den != "0"
}

fn validate_human_evidence(path: &str, line: usize, text: &str, known_tests: Option<&WorkspaceTests>) -> Option<EvidenceViolation> {
    let tokens = extract_candidate_tokens(text);
    let mut harness_hits = Vec::new();
    let mut valid_tests = Vec::new();
    let mut unknown_tests = Vec::new();

    for tok in tokens {
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
            String::from("missing passing human-path test name or recorded manual run")
        } else {
            format!("missing passing human-path test or recorded manual run (unmatched test identifier(s): {})", unknown_tests.join(", "))
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

fn is_wf_harness_fn(seg: &str) -> bool {
    let Some(rest) = seg.strip_prefix("wf") else {
        return false;
    };
    let digit_count = rest.chars().take_while(char::is_ascii_digit).count();
    digit_count > 0 && rest[digit_count..].starts_with('_')
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
    for prefix in ["manual:", "manual run:", "manual walkthrough:", "manual-verified:", "recorded manual:", "manual qa:"] {
        if let Some(pos) = lower.find(prefix)
            && !is_negated_prefix(&lower[..pos])
            && has_concrete_manual_detail(&lower[pos + prefix.len()..])
        {
            return true;
        }
    }
    ["manual (", "manual run (", "manual walkthrough ("].iter().any(|marker| {
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
        let violations = check_ui_parity_evidence("docs/ui-parity.md", md, Some(&tests));
        assert!(violations.is_empty(), "unexpected violations: {violations:?}");
    }

    #[test]
    fn ui_parity_rejects_missing_evidence_on_checked_item() {
        let md = "- [x] Interactive modal buttons work cleanly.\n";
        let violations = check_ui_parity_evidence("docs/ui-parity.md", md, None);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].line, 1);
        assert!(violations[0].reason.contains("missing passing human-path test"));
    }

    #[test]
    fn ui_parity_rejects_unknown_test_name_when_workspace_indexed() {
        let tests = WorkspaceTests::default();
        let md = "- [x] Modal dialog — verified by `ui_tests::invented_nonexistent_test`.\n";
        let violations = check_ui_parity_evidence("docs/ui-parity.md", md, Some(&tests));
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
            let violations = check_ui_parity_evidence("docs/ui-parity.md", md, Some(&tests));
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
        ];
        for md in cases {
            let violations = check_ui_parity_evidence("docs/ui-parity.md", md, None);
            assert_eq!(violations.len(), 1, "expected rejection for `{md}`, got {violations:?}");
        }
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
}
