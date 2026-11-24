#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Shared integration-test helpers (`TempDir`, `TempWorkspace`, `unique_temp_path`, command helpers,
//! `approx_eq`, and Base64 codecs backed by `curvebook_platform::base64`) for `crates/engine`.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_engine::{CommandOutcome, EngineError, Session};
use curvebook_testkit::assert_approx_eq;
use serde_json::Value;

static SEQ: AtomicU64 = AtomicU64::new(1);

pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> Self {
        Self::try_new(prefix).expect("create temp dir")
    }

    pub fn try_new(prefix: &str) -> Result<Self, String> {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-eng-{prefix}-{}-{seq}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path).map_err(|e| format!("create temp dir `{}`: {e}", path.display()))?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn child(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub struct TempWorkspace {
    pub root: PathBuf,
    pub workdir: PathBuf,
    pub outside: PathBuf,
}

impl TempWorkspace {
    pub fn new(label: &str) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-eng-ws-{label}-{}-{seq}", std::process::id()));
        let workdir = root.join("workdir");
        let outside = root.join("outside");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::create_dir_all(&outside).expect("create outside");
        Self { root, workdir, outside }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn unique_temp_path(label: &str, ext: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("cb-eng-{label}-{}-{seq}.{ext}", std::process::id()))
}

pub fn ok_cmd(session: &mut Session, id: &str, params: Value) -> CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

pub fn err_cmd(session: &mut Session, id: &str, params: Value) -> EngineError {
    match session.execute(id, params) {
        Ok(out) => panic!("command `{id}` unexpectedly succeeded: {:?}", out.payload),
        Err(err) => err,
    }
}

pub fn try_cmd(session: &mut Session, id: &str, params: Value) -> Result<CommandOutcome, Box<dyn Error>> {
    Ok(session.execute(id, params)?)
}

pub fn ok_payload(session: &mut Session, cmd: &str, params: Value) -> Result<Value, Box<dyn Error>> {
    Ok(session.execute(cmd, params)?.payload)
}

pub fn try_err_cmd(session: &mut Session, cmd: &str, params: Value) -> Result<EngineError, Box<dyn Error>> {
    match session.execute(cmd, params) {
        Ok(out) => Err(format!("expected `{cmd}` to fail, got payload: {:?}", out.payload).into()),
        Err(err) => Ok(err),
    }
}

pub fn approx_eq(actual: f64, expected: f64, tol: f64, label: &str) {
    assert_approx_eq(actual, expected, tol, tol, label);
}

pub fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

pub fn decode_base64(input: &str) -> Vec<u8> {
    curvebook_platform::base64::decode(input).expect("decode test base64")
}
