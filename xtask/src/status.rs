//! `cargo xtask status`: verify every Markdown table in `docs/STATUS.md` and `docs/ROADMAP.md`
//! has unique first-column keys (preventing silent merge-conflict duplicate rows).

use std::collections::BTreeSet;
use std::path::Path;

const STATUS_REL: &str = "docs/STATUS.md";
const ROADMAP_REL: &str = "docs/ROADMAP.md";

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

/// Validate Column-1 key uniqueness across every Markdown table in `docs/STATUS.md` and
/// `docs/ROADMAP.md`.
pub fn run(root: &Path) -> Result<(), String> {
    let mut total_tables = 0usize;
    let mut duplicates = Vec::new();

    for rel in [STATUS_REL, ROADMAP_REL] {
        let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let (tables, mut dups) = check_markdown_tables(rel, &text);
        if tables == 0 {
            return Err(format!("{rel}: expected at least one Markdown table"));
        }
        total_tables += tables;
        duplicates.append(&mut dups);
    }

    if duplicates.is_empty() {
        println!("status: verified {total_tables} Markdown table(s) across {STATUS_REL} and {ROADMAP_REL} (0 duplicate column-1 keys)");
        return Ok(());
    }
    let lines: Vec<String> = duplicates.iter().map(ToString::to_string).collect();
    Err(format!("{} duplicate table key(s) found:\n  {}", duplicates.len(), lines.join("\n  ")))
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
