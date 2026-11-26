//! Verify `docs/schemas/*.schema.json` matches `curvebook_format::schemas::all_persisted_schemas()`.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::Path;

use curvebook_format::schemas::all_persisted_schemas;

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn docs_schemas_match_format_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspace root");
    let dir = root.join("docs/schemas");
    let schemas = all_persisted_schemas();
    assert!(!schemas.is_empty());

    let mut expected_files = BTreeSet::new();
    for (name, value) in &schemas {
        let file_name = if name.ends_with(".schema.json") { (*name).to_owned() } else { format!("{name}.schema.json") };
        let path = dir.join(&file_name);
        expected_files.insert(file_name);
        let existing = std::fs::read_to_string(&path).expect("read committed schema file");
        let mut rendered = serde_json::to_string_pretty(value).expect("serialize schema");
        rendered.push('\n');
        assert_eq!(normalize_newlines(&existing), normalize_newlines(&rendered), "schema `{}` drifted; run `cargo xtask schemas`", path.display());
    }

    for entry in std::fs::read_dir(&dir).expect("read docs/schemas") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".schema.json") {
            assert!(expected_files.contains(&name), "unexpected schema file `{name}` in docs/schemas/");
        }
    }
}
