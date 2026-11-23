//! Regenerate or verify [`docs/commands.md`](../../docs/commands.md) from `curvebook_engine::cmd`.

use std::fs;
use std::path::Path;

use curvebook_platform::write_atomic;

const COMMANDS_DOC_REL: &str = "docs/commands.md";

/// Parse CLI flags for `cargo xtask commands [--check]`.
pub fn parse_args(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--check" => Ok(true),
        _ => Err(String::from("usage: cargo xtask commands [--check]")),
    }
}

/// CLI entrypoint for `cargo xtask commands [--check]`.
pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    let check = parse_args(args)?;
    run(root, check)
}

/// Regenerate or verify `docs/commands.md` against `curvebook_engine::cmd::render_commands_markdown()`.
pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let expected = normalize_newlines(&curvebook_engine::cmd::render_commands_markdown());
    let path = root.join(COMMANDS_DOC_REL);

    if check {
        let actual =
            fs::read_to_string(&path).map_err(|err| format!("commands: failed to read `{}` ({err}); run `cargo xtask commands`", path.display()))?;
        if normalize_newlines(&actual) != expected {
            return Err(format!("commands: `{}` is out of sync with curvebook_engine::cmd::registry(); run `cargo xtask commands`", path.display()));
        }
        println!("commands: {} is up to date", path.display());
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("commands: failed to create `{}`: {err}", parent.display()))?;
    }
    write_atomic(&path, expected.as_bytes()).map_err(|err| format!("commands: failed to write `{}`: {err}", path.display()))?;
    println!("commands: wrote {}", path.display());
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
            let dir = std::env::temp_dir().join(format!("curvebook-xtask-commands-{prefix}-{}-{seq}", std::process::id()));
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
    fn commands_write_check_and_drift_detection() {
        let tmp = TempDir::new("drift");
        assert!(run(&tmp.0, true).is_err(), "missing file must fail --check");

        run(&tmp.0, false).expect("generate docs/commands.md in tempdir");
        let generated_path = tmp.0.join("docs/commands.md");
        let contents = fs::read_to_string(&generated_path).expect("read generated commands.md");
        assert!(contents.starts_with("# CurveBook Command Reference\n"));
        let command_count = curvebook_engine::cmd::registry().len();
        assert!(command_count >= 92, "expected >= 92 registered commands, got {command_count}");
        assert!(contents.contains(&format!("Registered commands: {command_count}\n")));

        run(&tmp.0, true).expect("freshly generated docs/commands.md must pass --check");

        fs::write(&generated_path, format!("{contents}\n<!-- drifted -->\n")).expect("mutate commands.md");
        let err = run(&tmp.0, true).expect_err("mutated docs/commands.md must fail --check");
        assert!(err.contains("out of sync"), "unexpected error: {err}");
    }

    #[test]
    fn parse_args_validates_flags() {
        assert_eq!(parse_args(&[]), Ok(false));
        assert_eq!(parse_args(&[String::from("--check")]), Ok(true));
        assert!(parse_args(&[String::from("--unknown")]).is_err());

        let tmp = TempDir::new("cli");
        run_cli(&tmp.0, &[]).expect("run_cli write succeeds");
        run_cli(&tmp.0, &[String::from("--check")]).expect("run_cli check succeeds");
    }
}
