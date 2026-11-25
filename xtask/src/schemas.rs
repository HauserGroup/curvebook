//! Regenerate or verify `docs/schemas/*.schema.json` via `curvebook-format`'s `gen_schemas` example.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use curvebook_platform::write_atomic;

const USAGE: &str = "usage: cargo xtask schemas [--check]";

pub fn parse_args(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--check" => Ok(true),
        [flag] if flag == "-h" || flag == "--help" => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        [other, ..] => Err(format!("unknown `cargo xtask schemas` argument `{other}`\n{USAGE}")),
    }
}

pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    let check = parse_args(args)?;
    run(root, check)
}

pub fn render_schema_json(schema: &serde_json::Value) -> Result<String, String> {
    let mut rendered = serde_json::to_string_pretty(schema).map_err(|e| format!("serialize schema json: {e}"))?;
    rendered.push('\n');
    Ok(rendered)
}

pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let raw = crate::run_workspace_example(root, "curvebook-format", "gen_schemas", &[])?;
    let schemas: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&raw).map_err(|e| format!("parse gen_schemas JSON output: {e}"))?;
    let dir = root.join("docs/schemas");

    if check {
        return check_schemas(&dir, &schemas);
    }

    fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all {}: {e}", dir.display()))?;
    let mut expected_names = BTreeSet::new();
    for (name, schema) in &schemas {
        let file_name = schema_file_name(name);
        let path = dir.join(&file_name);
        let rendered = render_schema_json(schema)?;
        write_atomic(&path, rendered.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
        expected_names.insert(file_name);
    }
    check_no_unexpected_schema_files(&dir, &expected_names)?;
    println!("schemas: regenerated {} file(s) in {}", schemas.len(), dir.display());
    Ok(())
}

fn check_schemas(dir: &Path, schemas: &BTreeMap<String, serde_json::Value>) -> Result<(), String> {
    let mut expected_names = BTreeSet::new();
    for (name, schema) in schemas {
        let file_name = schema_file_name(name);
        let path = dir.join(&file_name);
        expected_names.insert(file_name);
        let existing =
            fs::read_to_string(&path).map_err(|e| format!("schemas: `{}` could not be read ({e}); run `cargo xtask schemas`", path.display()))?;
        let rendered = render_schema_json(schema)?;
        if normalize_newlines(&existing) != normalize_newlines(&rendered) {
            return Err(format!("schemas: `{}` is out of sync with curvebook_format::schemas; run `cargo xtask schemas`", path.display()));
        }
    }
    check_no_unexpected_schema_files(dir, &expected_names)?;
    println!("schemas: {} file(s) in {} match curvebook_format::schemas", schemas.len(), dir.display());
    Ok(())
}

fn schema_file_name(name: &str) -> String {
    if name.ends_with(".schema.json") { name.to_owned() } else { format!("{name}.schema.json") }
}

fn check_no_unexpected_schema_files(dir: &Path, expected_names: &BTreeSet<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry in {}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".schema.json") && !expected_names.contains(&name) {
            return Err(format!("schemas: unexpected extra schema file `{}` in {}; remove it or register it", name, dir.display()));
        }
    }
    Ok(())
}

use crate::scan_util::normalize_newlines;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::{check_no_unexpected_schema_files, parse_args};

    #[test]
    fn parse_args_accepts_default_or_check() {
        assert_eq!(parse_args(&[]), Ok(false));
        assert_eq!(parse_args(&[String::from("--check")]), Ok(true));
        assert!(parse_args(&[String::from("--bad")]).is_err());
    }

    #[test]
    fn committed_schema_files_match_expected_set() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
        let schemas_dir = root.join("docs/schemas");
        let expected_files: BTreeSet<String> =
            ["archive-manifest.schema.json", "batch-v1.schema.json", "document.schema.json", "journal-record.schema.json", "preferences.schema.json"]
                .into_iter()
                .map(str::to_owned)
                .collect();
        for name in &expected_files {
            assert!(schemas_dir.join(name).is_file(), "missing schema file {}", schemas_dir.join(name).display());
        }
        check_no_unexpected_schema_files(&schemas_dir, &expected_files).expect("unexpected schema file");
    }
}
