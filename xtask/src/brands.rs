//! `cargo xtask brands`: keep competitor vendor/product trademarks and retired suite branding out
//! of user-facing strings, package descriptions, workflows, and packaging metadata (`AGENTS.md`).

use std::path::Path;

/// Competitor vendor and product names checked on ASCII word boundaries (case-insensitive).
const BRANDS: &[&str] = &["graphpad", "dotmatics", "instat", "statmate", "jmp", "spss", "minitab", "originlab", "originpro", "sigmaplot", "systat"];

/// External suite branding and infrastructure forbidden across all non-xtask files in this standalone repository.
const SUITE_BRANDS: &[&str] = &[
    "artcraft",
    "getartcraft",
    "storytold",
    "storyteller",
    "craft-fonts",
    "craft_fonts",
    "craft_fonts_dir",
    "craft-common-actions",
    "craft-contrib",
    "craft-house-rules",
    "craft-ui",
    "craft_ui",
    "craft-keymap",
    "craft-update",
    "craft-window",
];

const ALLOW_MARKER: &str = "brand-ok";

#[derive(Debug, PartialEq, Eq)]
pub struct BrandHit {
    pub path: String,
    pub line: usize,
    pub brand: &'static str,
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let hits = scan_repo(root, &files);
    if hits.is_empty() {
        println!("brands: 0 unapproved brand or suite mentions across {} tracked files", files.len());
        return Ok(());
    }
    let lines: Vec<String> = hits.iter().map(|h| format!("{}:{}: `{}`", h.path, h.line, h.brand)).collect();
    Err(format!(
        "{} unapproved brand/suite mention(s) in user-facing, workflow, or packaging files (use neutral scientific wording, or mark intentional compatibility notes with `{ALLOW_MARKER}`):\n  {}",
        hits.len(),
        lines.join("\n  ")
    ))
}

#[must_use]
pub fn scan_repo(root: &Path, files: &[String]) -> Vec<BrandHit> {
    let mut hits = Vec::new();
    for path in files {
        if skip_path(path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        scan_file(path, &text, &mut hits);
    }
    hits
}

fn skip_path(path: &str) -> bool {
    path.starts_with("xtask/")
        || path.starts_with("crates/testkit/")
        || path.starts_with("tests/")
        || path.contains("/tests/")
        || path.ends_with("_test.rs")
}

fn scan_file(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    scan_suite_lines(path, text, out);
    let is_non_md_doc = path.starts_with("docs/") && !Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rs")) {
        scan_rust(path, text, out);
    } else if path.ends_with("Cargo.toml") {
        scan_cargo_toml(path, text, out);
    } else if path == "crates/tools/src/cursors.toml" || path.starts_with("packaging/") || path.starts_with(".github/workflows/") || is_non_md_doc {
        scan_plain_lines(path, text, out);
    }
}

fn scan_suite_lines(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        collect_line_words(path, idx + 1, line, SUITE_BRANDS, out);
    }
}

fn scan_cargo_toml(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("description") && !line.contains(ALLOW_MARKER) {
            collect_line_words(path, idx + 1, line, BRANDS, out);
        }
    }
}

fn scan_plain_lines(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    for (idx, &line) in lines.iter().enumerate() {
        if line.contains(ALLOW_MARKER) {
            continue;
        }
        let prev_allows = idx > 0 && lines.get(idx - 1).is_some_and(|prev| prev.trim_start().starts_with('#') && prev.contains(ALLOW_MARKER));
        if prev_allows {
            continue;
        }
        collect_line_words(path, idx + 1, line, BRANDS, out);
    }
}

fn scan_rust(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let prod = strip_cfg_test_tail(text);
    for (idx, line) in prod.lines().enumerate() {
        if line.contains(ALLOW_MARKER) {
            continue;
        }
        let code = strip_line_comment(line);
        for lit in extract_string_literals(code) {
            collect_line_words(path, idx + 1, &lit, BRANDS, out);
        }
    }
}

fn strip_cfg_test_tail(src: &str) -> &str {
    match src.find("#[cfg(test)]") {
        Some(pos) => &src[..pos],
        None => src,
    }
}

fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < bytes.len() {
        if in_str {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                in_str = false;
            }
        } else if bytes[i] == b'"' {
            in_str = true;
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn extract_string_literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = code.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut buf = String::new();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(esc) = chars.next() {
                    buf.push(esc);
                }
            } else if c == '"' {
                break;
            } else {
                buf.push(c);
            }
        }
        out.push(buf);
    }
    out
}

fn collect_line_words(path: &str, line_no: usize, haystack: &str, words: &[&'static str], out: &mut Vec<BrandHit>) {
    let lower = haystack.to_ascii_lowercase();
    for &brand in words {
        if contains_word(&lower, brand) {
            out.push(BrandHit { path: path.to_owned(), line: line_no, brand });
        }
    }
}

fn contains_word(lower_haystack: &str, word: &str) -> bool {
    let bytes = lower_haystack.as_bytes();
    let wlen = word.len();
    let mut start = 0;
    while let Some(rel) = lower_haystack[start..].find(word) {
        let pos = start + rel;
        let end = pos + wlen;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{BrandHit, scan_file};

    #[test]
    fn flags_rust_string_literals_but_ignores_comments_and_cfg_test() {
        let mut hits = Vec::new();
        let src = "\
// GraphPad comment is fine
pub const LABEL: &str = \"Import GraphPad table\";
pub const OK: &str = \"Import OriginPro table\"; // brand-ok
pub fn instantiate() -> &'static str { \"instantiate\" }
#[cfg(test)]
mod tests {
    const T: &str = \"SPSS fixture\";
}
";
        scan_file("crates/ui-egui/src/lib.rs", src, &mut hits);
        assert_eq!(hits, vec![BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 2, brand: "graphpad" }]);
    }

    #[test]
    fn desktop_entry_honors_preceding_comment_allow_marker() {
        let mut hits = Vec::new();
        let desktop = "\
[Desktop Entry]
Name=LumenCraft
# Compatibility search term: brand-ok
Keywords=biostatistics;regression;GraphPad;
";
        scan_file("packaging/linux/org.lumencraft.LumenCraft.desktop", desktop, &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn flags_artcraft_suite_references_in_docs_and_workflows() {
        let mut hits = Vec::new();
        scan_file("docs/releasing.md", "Installs under Programs\\ArtCraft\\LumenCraft or https://getartcraft.com\n", &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "getartcraft" },
            ]
        );
    }
}
