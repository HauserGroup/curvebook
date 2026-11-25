//! Shared temporary-directory, CLI-runner, and archive fixture helpers for `xtask_suite`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct TempDirGuard {
    pub path: PathBuf,
}

impl TempDirGuard {
    pub fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-xtask-suite-{label}-{}-{nanos}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).map_err(|e| format!("create_dir_all {}: {e}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

pub fn run_xtask_in(cwd: &Path, args: &[&str], envs: &[(&str, Option<&str>)]) -> Result<Output, String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xtask"));
    cmd.current_dir(cwd).args(args);
    for &(k, v) in envs {
        match v {
            Some(val) => {
                cmd.env(k, val);
            }
            None => {
                cmd.env_remove(k);
            }
        }
    }
    cmd.output().map_err(|e| format!("run xtask {args:?}: {e}"))
}

pub fn run_xtask(args: &[&str], envs: &[(&str, Option<&str>)]) -> Result<Output, String> {
    run_xtask_in(&workspace_root()?, args, envs)
}

pub fn run_release_scan(args: &[&str]) -> Result<Output, String> {
    let mut full = Vec::with_capacity(args.len() + 1);
    full.push("release-scan");
    full.extend_from_slice(args);
    run_xtask(&full, &[])
}

/// Encode raw bytes as a valid RFC 1952 `.gz` stream using DEFLATE stored blocks (method 8, BTYPE=00).
pub fn gzip_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 32);
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03]);
    let chunks: Vec<&[u8]> = if raw.is_empty() { vec![&[]] } else { raw.chunks(65_535).collect() };
    for (idx, chunk) in chunks.iter().enumerate() {
        let bfinal = u8::from(idx + 1 == chunks.len());
        let len = chunk.len() as u16;
        out.push(bfinal);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32fast::hash(raw).to_le_bytes());
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out
}

/// Build a single 512-byte POSIX ustar header block (regular file, symlink, or hardlink).
pub fn build_ustar_header(path: &str, typeflag: u8, size: usize, linkname: &str) -> Result<[u8; 512], String> {
    let mut hdr = [0u8; 512];
    let p_bytes = path.as_bytes();
    if p_bytes.len() > 100 {
        return Err(format!("test tar path > 100 bytes: {path}"));
    }
    hdr[..p_bytes.len()].copy_from_slice(p_bytes);
    hdr[100..108].copy_from_slice(b"0000644\0");
    hdr[108..116].copy_from_slice(b"0000000\0");
    hdr[116..124].copy_from_slice(b"0000000\0");
    hdr[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    hdr[136..148].copy_from_slice(b"00000000000\0");
    hdr[148..156].fill(b' ');
    hdr[156] = typeflag;
    let l_bytes = linkname.as_bytes();
    let l_len = l_bytes.len().min(100);
    hdr[157..157 + l_len].copy_from_slice(&l_bytes[..l_len]);
    hdr[257..265].copy_from_slice(b"ustar\x0000");
    let chksum: u32 = hdr.iter().map(|&b| u32::from(b)).sum();
    hdr[148..156].copy_from_slice(format!("{chksum:06o}\0 ").as_bytes());
    Ok(hdr)
}

/// Build a POSIX ustar `.tar` archive from `(path, bytes)` entries (allows arbitrary paths for tar-slip negative tests).
pub fn build_ustar_bytes(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    let mut tar = Vec::new();
    for &(name, data) in entries {
        tar.extend_from_slice(&build_ustar_header(name, b'0', data.len(), "")?);
        tar.extend_from_slice(data);
        tar.resize(tar.len() + (512 - (data.len() % 512)) % 512, 0);
    }
    tar.resize(tar.len() + 1024, 0);
    Ok(tar)
}

pub fn write_sha256sums_for_dir(dir: &Path) -> Result<(), String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let name = entry.map_err(|e| e.to_string())?.file_name().to_string_lossy().into_owned();
        if name != "SHA256SUMS.txt" {
            names.push(name);
        }
    }
    names.sort();
    let mut lines = String::new();
    for name in names {
        let bytes = fs::read(dir.join(&name)).map_err(|e| e.to_string())?;
        lines.push_str(&format!("{}  {name}\n", curvebook_platform::sha256::sha256_hex(&bytes)));
    }
    fs::write(dir.join("SHA256SUMS.txt"), lines).map_err(|e| e.to_string())
}
