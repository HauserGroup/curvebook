//! Verify `docs/commands.md` matches `curvebook_engine::cmd::render_commands_markdown()`.
#![forbid(unsafe_code)]

use std::path::Path;

use curvebook_engine::cmd::{registry, render_commands_markdown};

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn docs_commands_md_matches_engine_registry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspace root");
    let doc_path = root.join("docs/commands.md");
    let existing = std::fs::read_to_string(&doc_path).expect("read docs/commands.md");
    let expected = render_commands_markdown();
    assert_eq!(
        normalize_newlines(&existing),
        normalize_newlines(&expected),
        "docs/commands.md is out of sync with curvebook_engine::cmd; run `cargo xtask commands`"
    );
    assert!(expected.contains(&format!("Registered commands: {}", registry().len())));
}
