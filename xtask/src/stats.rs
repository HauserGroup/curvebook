//! `cargo xtask stats [--exact]`: count Rust files, lines of code, `#[test]` attributes, and
//! `proptest!` invocations across every workspace package.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::layers;
use crate::rust_lex::{blank_comments_and_literals_both, find_matching_delim, prev_is_ident};
use crate::scan_util::{has_ext_ci, walk_file_paths};

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
        let mut files = Vec::new();
        walk_file_paths(
            &pkg_dir,
            &|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|s| s == "target" || s.starts_with('.')),
            &|p| has_ext_ci(p, &["rs"]),
            &mut files,
        )?;
        for path in files {
            let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if exact {
                accumulate_source_exact(&src, &mut st);
            } else {
                accumulate_source(&src, &mut st);
            }
        }
        rows.push(st);
    }
    rows.sort_by(|a, b| a.layer.cmp(&b.layer).then_with(|| a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn accumulate_source(src: &str, st: &mut PkgStats) {
    st.files += 1;
    for line in src.lines() {
        st.lines += 1;
        let t = line.trim();
        st.tests += usize::from(t.starts_with("#[test]"));
        st.proptests += usize::from(t.starts_with("proptest!"));
    }
}

pub fn accumulate_source_exact(src: &str, st: &mut PkgStats) {
    st.files += 1;
    let stripped = blank_comments_and_literals_both(src, false).0;
    st.lines += stripped.lines().filter(|l| !l.trim().is_empty()).count();
    let (bytes, mut i) = (stripped.as_bytes(), 0usize);
    while i + 1 < bytes.len() {
        if bytes[i] == b'#' && bytes[i + 1] == b'[' {
            let end = find_matching_delim(bytes, i + 1, b'[', b']').map_or(bytes.len(), |c| c + 1);
            let attr: String = stripped.get(i..end).unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
            st.tests += usize::from(attr == "#[test]" || attr.contains("::test]") || attr.contains("::test("));
            i = end;
        } else {
            i += 1;
        }
    }
    st.proptests += stripped.match_indices("proptest!").filter(|&(pos, _)| !prev_is_ident(bytes, pos)).count();
}

fn print_table(rows: &[PkgStats], exact: bool) {
    println!("{:<24} {:<10} {:>6} {:>8} {:>7} {:>9}", "package", "layer", "files", if exact { "sloc" } else { "lines" }, "tests", "proptests");
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
    use super::{PkgStats, accumulate_source, accumulate_source_exact};

    #[test]
    fn counts_lines_tests_proptests_and_exact_stripping() {
        let mut st = PkgStats::default();
        accumulate_source("fn a() {}\n#[test]\nfn t() {}\nproptest! {\n}\n", &mut st);
        assert_eq!((st.files, st.lines, st.tests, st.proptests), (1, 5, 1, 1));
        assert_eq!(crate::layers::describe(crate::layers::classify("curvebook-platform")), "L0");

        let src = r##"
// #[test] comment fake
/* #[test] proptest! {} */
const ESC_UTF8: &str = "\α#[test]proptest! {}";
const RAW: &str = r#"
#[test]
proptest! {}
"#;
#[test]
fn real_one() {}
"##;
        let (mut approx, mut exact) = (PkgStats::default(), PkgStats::default());
        accumulate_source(src, &mut approx);
        accumulate_source_exact(src, &mut exact);
        assert!(approx.tests > exact.tests && exact.tests == 1 && exact.proptests == 0 && exact.lines < approx.lines);

        let mut trailing = PkgStats::default();
        accumulate_source_exact("const S: &str = \"\\α", &mut trailing);
        assert_eq!(trailing.tests, 0);
    }
}
