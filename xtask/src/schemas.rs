//! Regenerate or verify `docs/schemas/*.schema.json` from `lumencraft_format::schemas`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SCHEMAS_DIR_REL: &str = "docs/schemas";

/// Parse CLI flags for `cargo xtask schemas [--check]`.
pub fn parse_args(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--check" => Ok(true),
        _ => Err(String::from("usage: cargo xtask schemas [--check]")),
    }
}

/// CLI entrypoint for `cargo xtask schemas [--check]`.
pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    let check = parse_args(args)?;
    run(root, check)
}

/// Format a schema JSON object consistently with a trailing newline.
pub fn render_schema_json(schema: &serde_json::Value) -> Result<String, String> {
    let mut rendered = serde_json::to_string_pretty(schema).map_err(|err| format!("schemas: serialize JSON failed: {err}"))?;
    rendered = normalize_newlines(&rendered);
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

/// Regenerate or verify all persisted format schemas under `docs/schemas/`.
pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let schemas = lumencraft_format::schemas::all_persisted_schemas();
    if schemas.is_empty() {
        return Err(String::from("schemas: all_persisted_schemas() returned zero entries"));
    }

    let dir = root.join(SCHEMAS_DIR_REL);
    if check {
        return check_schemas(&dir, &schemas);
    }

    fs::create_dir_all(&dir).map_err(|err| format!("schemas: failed to create `{}`: {err}", dir.display()))?;
    for (name, value) in &schemas {
        let rendered = render_schema_json(value)?;
        let out_path = dir.join(name);
        fs::write(&out_path, rendered.as_bytes()).map_err(|err| format!("schemas: failed to write `{}`: {err}", out_path.display()))?;
    }
    println!("schemas: wrote {} schema file(s) to {}", schemas.len(), dir.display());
    Ok(())
}

fn check_schemas(dir: &Path, schemas: &[(&'static str, serde_json::Value)]) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("schemas: missing directory `{}`; run `cargo xtask schemas`", dir.display()));
    }

    let mut expected_names = BTreeSet::new();
    for (name, value) in schemas {
        expected_names.insert(*name);
        let expected = render_schema_json(value)?;
        let path = dir.join(name);
        let actual =
            fs::read_to_string(&path).map_err(|err| format!("schemas: failed to read `{}` ({err}); run `cargo xtask schemas`", path.display()))?;
        if normalize_newlines(&actual) != expected {
            return Err(format!("schemas: `{}` is out of sync with lumencraft_format::schemas; run `cargo xtask schemas`", path.display()));
        }
    }

    check_no_unexpected_schema_files(dir, &expected_names)?;
    println!("schemas: {} schema file(s) in {} are up to date", schemas.len(), dir.display());
    Ok(())
}

fn check_no_unexpected_schema_files(dir: &Path, expected_names: &BTreeSet<&str>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|err| format!("schemas: failed to read `{}`: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("schemas: read_dir entry error: {err}"))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            return Err(format!("schemas: non-UTF8 filename in `{}`", dir.display()));
        };
        if name.ends_with(".schema.json") && !expected_names.contains(name) {
            return Err(format!("schemas: unexpected stale schema `{}` in `{}`", name, dir.display()));
        }
    }
    Ok(())
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{parse_args, run, run_cli};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("lumencraft-xtask-schemas-{prefix}-{}-{seq}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create tempdir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn schemas_write_check_and_drift_detection() {
        let expected_files =
            ["archive-manifest.schema.json", "document.schema.json", "batch-v1.schema.json", "preferences.schema.json", "journal-record.schema.json"];
        assert_eq!(lumencraft_format::schemas::all_persisted_schemas().len(), expected_files.len());

        let tmp = TempDir::new("drift");
        assert!(run(&tmp.0, true).is_err(), "missing dir must fail --check");

        run(&tmp.0, false).expect("generate schemas in tempdir");
        for file in expected_files {
            let path = tmp.0.join("docs/schemas").join(file);
            let text = fs::read_to_string(&path).expect("read generated schema");
            assert!(text.ends_with('\n'), "{file} must end with trailing newline");
            let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid schema json");
            assert!(parsed.is_object(), "{file} root schema must be a JSON object");
        }

        run(&tmp.0, true).expect("freshly generated schemas must pass --check");

        // Mutate one schema file and verify --check fails.
        let mutated_path = tmp.0.join("docs/schemas/document.schema.json");
        let original = fs::read_to_string(&mutated_path).expect("read document schema");
        fs::write(&mutated_path, "{}\n").expect("mutate document schema");
        let err = run(&tmp.0, true).expect_err("mutated schema must fail --check");
        assert!(err.contains("out of sync"), "unexpected error: {err}");

        // Restore mutated file and delete another schema file to verify missing file fails.
        fs::write(&mutated_path, original).expect("restore document schema");
        fs::remove_file(tmp.0.join("docs/schemas/preferences.schema.json")).expect("delete preferences schema");
        assert!(run(&tmp.0, true).is_err(), "deleted schema must fail --check");
    }

    #[test]
    fn parse_args_validates_flags() {
        assert_eq!(parse_args(&[]), Ok(false));
        assert_eq!(parse_args(&[String::from("--check")]), Ok(true));
        assert!(parse_args(&[String::from("--bogus")]).is_err());

        let tmp = TempDir::new("cli");
        run_cli(&tmp.0, &[]).expect("run_cli write succeeds");
        run_cli(&tmp.0, &[String::from("--check")]).expect("run_cli check succeeds");
    }
}
