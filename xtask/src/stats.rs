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
    let rows = collect_workspace_stats(&meta)?;
    print_table(&rows, exact);
    Ok(())
}

fn collect_workspace_stats(meta: &Value) -> Result<Vec<PkgStats>, String> {
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or_else(|| String::from("metadata: missing packages"))?;
    let mut rows = Vec::with_capacity(pkgs.len());
    for pkg in pkgs {
        let name = pkg.get("name").and_then(Value::as_str).ok_or_else(|| String::from("metadata: package without name"))?.to_owned();
        let manifest = pkg.get("manifest_path").and_then(Value::as_str).ok_or_else(|| format!("{name}: missing manifest_path"))?;
        let pkg_dir = Path::new(manifest).parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let mut st = PkgStats { layer: layers::describe(layers::classify(&name)), name, ..PkgStats::default() };
        walk_rs(&pkg_dir, &mut st)?;
        rows.push(st);
    }
    rows.sort_by(|a, b| a.layer.cmp(&b.layer).then_with(|| a.name.cmp(&b.name)));
    Ok(rows)
}

fn walk_rs(dir: &Path, st: &mut PkgStats) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
        let path = entry.path();
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        if path.is_dir() {
            if fname_str == "target" || fname_str.starts_with('.') {
                continue;
            }
            walk_rs(&path, st)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            accumulate_source(&src, st);
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

fn print_table(rows: &[PkgStats], _exact: bool) {
    println!("{:<24} {:<10} {:>6} {:>8} {:>7} {:>9}", "package", "layer", "files", "lines", "tests", "proptests");
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
}
