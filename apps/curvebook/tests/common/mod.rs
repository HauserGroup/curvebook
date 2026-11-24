//! Shared integration test helpers for `curvebook` desktop suites.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

/// RAII temporary directory removed automatically on drop.
#[derive(Debug)]
pub struct TempDirGuard {
    pub path: PathBuf,
}

impl TempDirGuard {
    /// Create a fresh uniquely-named directory under `std::env::temp_dir()`.
    pub fn new(prefix: &str) -> std::io::Result<Self> {
        let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-{prefix}-{}-{seq}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// Borrow the directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Build a child path inside the temporary directory.
    #[must_use]
    pub fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Encode bytes as standard Base64 via `curvebook_platform::base64`.
#[must_use]
pub fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

/// Decode standard Base64 via `curvebook_platform::base64`.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    curvebook_platform::base64::decode(input).map_err(|err| err.to_string())
}

/// Parse `(width, height)` from a PNG IHDR header.
#[must_use]
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let chunk = png.get(12..24)?;
    let (ihdr, dims) = chunk.split_at_checked(4)?;
    if ihdr != b"IHDR" {
        return None;
    }
    let (w_bytes, h_bytes) = dims.split_at_checked(4)?;
    let w = u32::from_be_bytes(w_bytes.try_into().ok()?);
    let h = u32::from_be_bytes(h_bytes.try_into().ok()?);
    Some((w, h))
}
