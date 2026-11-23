//! `cargo xtask pending [--count]`: list open `- [ ]` roadmap items from `docs/ROADMAP.md`
//! grouped by their enclosing `### R<m>.<n>` heading.

use std::collections::BTreeSet;
use std::path::Path;

const ROADMAP_REL: &str = "docs/ROADMAP.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingItem {
    pub item_id: String,
    pub line: usize,
    pub text: String,
}

/// Print open `- [ ]` items in `docs/ROADMAP.md` (or total count with `--count`).
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    match args {
        [] => {
            let items = load_pending(root)?;
            for item in &items {
                println!("[{}] {} ({ROADMAP_REL}:{})", item.item_id, item.text, item.line);
            }
            let unique_items = distinct_roadmap_items(&items);
            println!("pending: {} open sub-task(s) across {} roadmap item(s)", items.len(), unique_items);
            Ok(())
        }
        [flag] if flag == "--count" => run_count(root),
        other => Err(format!("usage: cargo xtask pending [--count] (unexpected args: `{}`)", other.join(" "))),
    }
}

/// Print the total number of pending `- [ ]` sub-tasks in `docs/ROADMAP.md`.
pub fn run_count(root: &Path) -> Result<(), String> {
    let items = load_pending(root)?;
    let unique_items = distinct_roadmap_items(&items);
    println!("pending: {} open sub-task(s) across {} roadmap item(s)", items.len(), unique_items);
    Ok(())
}

fn load_pending(root: &Path) -> Result<Vec<PendingItem>, String> {
    let md = std::fs::read_to_string(root.join(ROADMAP_REL)).map_err(|e| format!("{ROADMAP_REL}: {e}"))?;
    Ok(collect_pending(&md))
}

fn distinct_roadmap_items(items: &[PendingItem]) -> usize {
    items.iter().map(|i| i.item_id.as_str()).collect::<BTreeSet<_>>().len()
}

/// Parse `docs/ROADMAP.md` text and return every `- [ ]` task tagged with its active `R<m>.<n>` ID.
#[must_use]
pub fn collect_pending(md: &str) -> Vec<PendingItem> {
    let mut out = Vec::new();
    let mut active_id: Option<String> = None;
    let mut in_fence = false;

    for (idx, line) in md.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("### ") {
            active_id = extract_roadmap_id(heading).map(str::to_owned);
            continue;
        }
        if trimmed.starts_with("## ") && !trimmed.starts_with("### ") {
            active_id = None;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- [ ]") {
            let text = rest.trim();
            if !text.is_empty() {
                let item_id = active_id.clone().unwrap_or_else(|| String::from("unscoped"));
                out.push(PendingItem { item_id, line: idx + 1, text: text.to_owned() });
            }
        }
    }
    out
}

fn extract_roadmap_id(heading: &str) -> Option<&str> {
    let raw = heading.split_whitespace().next()?;
    let token = raw.trim_start_matches(['`', '(']).trim_end_matches(['`', ')', ':', '.', ',', ';', '-']);
    let body = token.strip_prefix('R')?;
    let (major, minor) = body.split_once('.')?;
    if !major.is_empty() && major.bytes().all(|b| b.is_ascii_digit()) && !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit()) {
        Some(token)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingItem, collect_pending};

    #[test]
    fn tracks_active_roadmap_headings_and_ignores_completed_or_fenced_items() {
        let md = "\
### R0.3 Secrets gate
- [x] Hook added
- [ ] Implement secrets command

### `R5.4`: Native clipboard
- [ ] Implement NSPasteboard adapter
```markdown
- [ ] Not a real roadmap checkbox
```

### R6.1, Windows installer
- [ ] Sign MSI

## Parked
- [ ] Deferred outside R headings
";
        let got = collect_pending(md);
        assert_eq!(
            got,
            vec![
                PendingItem { item_id: String::from("R0.3"), line: 3, text: String::from("Implement secrets command") },
                PendingItem { item_id: String::from("R5.4"), line: 6, text: String::from("Implement NSPasteboard adapter") },
                PendingItem { item_id: String::from("R6.1"), line: 12, text: String::from("Sign MSI") },
                PendingItem { item_id: String::from("unscoped"), line: 15, text: String::from("Deferred outside R headings") },
            ]
        );
    }
}
