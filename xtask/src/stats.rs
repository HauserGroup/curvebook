//! `cargo xtask stats [--exact]`: count Rust files, lines of code, `#[test]` attributes, and
//! `proptest!` invocations across every workspace package.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::layers;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PkgStats {
    pub name: String,
    pub layer: String,
    pub files: usize,
    pub lines: usize,
    pub tests: usize,
    pub proptests: usize,
}

pub fn run(root: &Path, exact: bool) -> Result<(), String> {
    let meta = crate::cargo_metadata(root)?;
    let rows = collect_workspace_stats(&meta, exact)?;
    print_table(&rows, exact);
    Ok(())
}

fn collect_workspace_stats(meta: &Value, exact: bool) -> Result<Vec<PkgStats>, String> {
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or_else(|| String::from("metadata: missing packages"))?;
    let mut rows = Vec::with_capacity(pkgs.len());
    for pkg in pkgs {
        let name = pkg.get("name").and_then(Value::as_str).ok_or_else(|| String::from("metadata: package without name"))?.to_owned();
        let manifest = pkg.get("manifest_path").and_then(Value::as_str).ok_or_else(|| format!("{name}: missing manifest_path"))?;
        let pkg_dir = Path::new(manifest).parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let mut st = PkgStats { layer: layers::describe(layers::classify(&name)), name, ..PkgStats::default() };
        walk_rs(&pkg_dir, exact, &mut st)?;
        rows.push(st);
    }
    rows.sort_by(|a, b| a.layer.cmp(&b.layer).then_with(|| a.name.cmp(&b.name)));
    Ok(rows)
}

fn walk_rs(dir: &Path, exact: bool, st: &mut PkgStats) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        if ft.is_dir() {
            if fname_str == "target" || fname_str.starts_with('.') {
                continue;
            }
            walk_rs(&path, exact, st)?;
        } else if ft.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if exact {
                accumulate_source_exact(&src, st);
            } else {
                accumulate_source(&src, st);
            }
        }
    }
    Ok(())
}

pub fn accumulate_source(src: &str, st: &mut PkgStats) {
    st.files += 1;
    for line in src.lines() {
        st.lines += 1;
        let t = line.trim();
        if t == "#[test]" || t.starts_with("#[test]") {
            st.tests += 1;
        }
        if t.starts_with("proptest!") {
            st.proptests += 1;
        }
    }
}

/// Exact comment- and string-stripped accumulation of non-empty code lines, `#[test]` attributes,
/// and `proptest!` macro invocations.
pub fn accumulate_source_exact(src: &str, st: &mut PkgStats) {
    st.files += 1;
    let stripped = strip_rust_comments_and_strings(src);
    for line in stripped.lines() {
        if !line.trim().is_empty() {
            st.lines += 1;
        }
    }
    st.tests += count_exact_test_attrs(&stripped);
    st.proptests += count_exact_word_macro(&stripped, "proptest!");
}

fn strip_rust_comments_and_strings(src: &str) -> String {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = String::with_capacity(n);
    let mut i = 0usize;
    while i < n {
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
            out.push_str("  ");
            i += 2;
            let mut depth = 1usize;
            while i < n && depth > 0 {
                if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if i + 1 < n && b[i] == b'*' && b[i + 1] == b'/' {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                } else if b[i] == b'\n' {
                    out.push('\n');
                    i += 1;
                } else {
                    out.push(' ');
                    i += next_char_at(src, i).map_or(1, char::len_utf8);
                }
            }
            continue;
        }
        if let Some(raw_end) = skip_raw_string_end(b, i) {
            for &byte in &b[i..raw_end] {
                out.push(if byte == b'\n' { '\n' } else { ' ' });
            }
            i = raw_end;
            continue;
        }
        if b[i] == b'"' {
            out.push(' ');
            i += 1;
            while i < n {
                if b[i] == b'\\' {
                    out.push(' ');
                    if let Some(esc) = next_char_at(src, i + 1) {
                        out.push(if esc == '\n' { '\n' } else { ' ' });
                        i += 1 + esc.len_utf8();
                    } else {
                        i = n;
                    }
                } else if b[i] == b'"' {
                    out.push(' ');
                    i += 1;
                    break;
                } else if b[i] == b'\n' {
                    out.push('\n');
                    i += 1;
                } else {
                    out.push(' ');
                    i += next_char_at(src, i).map_or(1, char::len_utf8);
                }
            }
            continue;
        }
        if b[i] == b'\''
            && let Some(char_end) = skip_char_end(src, i)
        {
            for &byte in &b[i..char_end] {
                out.push(if byte == b'\n' { '\n' } else { ' ' });
            }
            i = char_end;
            continue;
        }
        let ch = next_char_at(src, i).unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn next_char_at(src: &str, idx: usize) -> Option<char> {
    src.get(idx..).and_then(|s| s.chars().next())
}

fn skip_raw_string_end(b: &[u8], idx: usize) -> Option<usize> {
    let mut pos = idx;
    if matches!(b.get(pos), Some(b'b' | b'c')) && b.get(pos + 1) == Some(&b'r') {
        pos += 1;
    }
    if b.get(pos) != Some(&b'r') {
        return None;
    }
    pos += 1;
    let mut hashes = 0usize;
    while b.get(pos) == Some(&b'#') {
        hashes += 1;
        pos += 1;
    }
    if b.get(pos) != Some(&b'"') {
        return None;
    }
    pos += 1;
    while pos < b.len() {
        if b[pos] == b'"' && b.get(pos + 1..pos + 1 + hashes).is_some_and(|s| s.iter().all(|&c| c == b'#')) {
            return Some(pos + 1 + hashes);
        }
        pos += 1;
    }
    Some(b.len())
}

fn skip_char_end(src: &str, idx: usize) -> Option<usize> {
    let tail = src.get(idx + 1..)?;
    let mut chars = tail.char_indices();
    let (_, first) = chars.next()?;
    if first == '\\' {
        let _ = chars.next()?;
        for (rel, ch) in chars {
            if ch == '\'' {
                return Some(idx + 1 + rel + 1);
            }
            if ch == '\n' || rel > 12 {
                return None;
            }
        }
        return None;
    }
    let (rel, next) = chars.next()?;
    if first != '\'' && next == '\'' { Some(idx + 1 + rel + 1) } else { None }
}

fn count_exact_test_attrs(code: &str) -> usize {
    let mut count = 0usize;
    let mut i = 0usize;
    let bytes = code.as_bytes();
    while i + 1 < bytes.len() {
        if bytes[i] == b'#' && bytes[i + 1] == b'[' {
            let mut depth = 0isize;
            let start = i;
            let mut end = bytes.len();
            let mut j = i + 1;
            while j < bytes.len() {
                match bytes[j] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            end = j + 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            let attr: String = code.get(start..end).unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
            if attr == "#[test]" || attr.contains("::test]") || attr.contains("::test(") {
                count += 1;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    count
}

fn count_exact_word_macro(code: &str, macro_call: &str) -> usize {
    let bytes = code.as_bytes();
    let mut count = 0usize;
    for (pos, _) in code.match_indices(macro_call) {
        let left_ok = pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_');
        if left_ok {
            count += 1;
        }
    }
    count
}

fn print_table(rows: &[PkgStats], exact: bool) {
    let lines_col = if exact { "sloc" } else { "lines" };
    println!("{:<24} {:<10} {:>6} {:>8} {:>7} {:>9}", "package", "layer", "files", lines_col, "tests", "proptests");
    println!("{}", "-".repeat(69));
    let mut tot = PkgStats::default();
    for r in rows {
        println!("{:<24} {:<10} {:>6} {:>8} {:>7} {:>9}", r.name, r.layer, r.files, r.lines, r.tests, r.proptests);
        tot.files += r.files;
        tot.lines += r.lines;
        tot.tests += r.tests;
        tot.proptests += r.proptests;
    }
    println!("{}", "-".repeat(69));
    println!("{:<24} {:<10} {:>6} {:>8} {:>7} {:>9}", "TOTAL", "", tot.files, tot.lines, tot.tests, tot.proptests);
}

#[cfg(test)]
mod tests {
    use super::{PkgStats, accumulate_source};

    #[test]
    fn counts_lines_tests_and_proptests() {
        let mut st = PkgStats::default();
        accumulate_source("fn a() {}\n#[test]\nfn t() {}\nproptest! {\n}\n", &mut st);
        assert_eq!(st.files, 1);
        assert_eq!(st.lines, 5);
        assert_eq!(st.tests, 1);
        assert_eq!(st.proptests, 1);
    }

    #[test]
    fn classifies_platform_crate_as_layer_zero() {
        assert_eq!(crate::layers::describe(crate::layers::classify("curvebook-platform")), "L0");
    }

    #[test]
    fn fa_sec23_12_exact_mode_strips_comments_and_strings() {
        let src = r##"
// #[test] comment fake
/*
#[test]
proptest! {}
*/
const ESC_UTF8: &str = "\α#[test]proptest! {}";
const RAW: &str = r#"
#[test]
proptest! {}
"#;
#[test]
fn real_one() {}
"##;
        let mut approx = PkgStats::default();
        accumulate_source(src, &mut approx);
        let mut exact = PkgStats::default();
        super::accumulate_source_exact(src, &mut exact);
        assert!(approx.tests > exact.tests, "approx={approx:?}, exact={exact:?}");
        assert_eq!(exact.tests, 1);
        assert_eq!(exact.proptests, 0);
        assert!(exact.lines < approx.lines);

        let mut trailing = PkgStats::default();
        super::accumulate_source_exact("const S: &str = \"\\α", &mut trailing);
        assert_eq!(trailing.tests, 0);
    }
}
