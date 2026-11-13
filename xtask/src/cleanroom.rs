//! `cargo xtask cleanroom`: ensure no vendor copyright notices or vendor-authored project
//! artifacts are committed to the repository (`AGENTS.md` › Non-negotiables).

use std::path::Path;

/// Forbidden vendor copyright or creator signatures in source/test fixtures.
const FORBIDDEN_SIGNATURES: &[&str] = &[
    "copyright (c) graphpad",
    "copyright graphpad software",
    "all rights reserved. graphpad",
    "dotmatics limited. all rights reserved",
    "<GraphPadPrismFile",
    "Created by GraphPad",
];

const SELF: &str = "xtask/src/cleanroom.rs";

#[derive(Debug, PartialEq, Eq)]
pub struct CleanroomHit {
    pub path: String,
    pub line: usize,
    pub signature: &'static str,
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let hits = scan(root, &files);
    if hits.is_empty() {
        println!("cleanroom: no vendor copyright notices or vendor-created fixtures found");
        return Ok(());
    }
    let formatted: Vec<String> = hits.iter().map(|h| format!("{}:{}: matched `{}`", h.path, h.line, h.signature)).collect();
    Err(format!("{} clean-room violation(s) found:\n  {}", hits.len(), formatted.join("\n  ")))
}

#[must_use]
pub fn scan(root: &Path, files: &[String]) -> Vec<CleanroomHit> {
    let mut hits = Vec::new();
    for path in files {
        if path == SELF {
            continue;
        }
        let Ok(bytes) = std::fs::read(root.join(path)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for (idx, line) in text.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            for &sig in FORBIDDEN_SIGNATURES {
                let sig_match = if sig.starts_with('<') || sig.starts_with("Created ") { line.contains(sig) } else { lower.contains(sig) };
                if sig_match {
                    hits.push(CleanroomHit { path: path.clone(), line: idx + 1, signature: sig });
                }
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::scan;

    #[test]
    fn flags_vendor_xml_fixture_and_copyright() {
        let dir = std::env::temp_dir().join(format!("lc-cleanroom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.xml"), "<?xml version=\"1.0\"?>\n<GraphPadPrismFile PrismXMLVersion=\"5.00\"/>\n").unwrap();
        std::fs::write(dir.join("ok.rs"), "// Clean-room Rust implementation\n").unwrap();
        let files = vec![String::from("bad.xml"), String::from("ok.rs")];
        let hits = scan(&dir, &files);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "bad.xml");
        assert_eq!(hits[0].line, 2);
    }
}
