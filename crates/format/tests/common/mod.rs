use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

pub struct TempDirGuard {
    path: PathBuf,
    #[cfg(unix)]
    restore_writable: Vec<PathBuf>,
}

impl TempDirGuard {
    #[allow(dead_code)]
    pub fn new(label: &str) -> std::io::Result<Self> {
        let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let clean: String = label.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
        let dir_name = if clean.is_empty() {
            format!("cb-format-{}-{seq}", std::process::id())
        } else {
            format!("cb-format-{clean}-{}-{seq}", std::process::id())
        };
        let path = std::env::temp_dir().join(dir_name);
        if path.exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            #[cfg(unix)]
            restore_writable: Vec::new(),
        })
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[allow(dead_code)]
    pub fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    #[cfg(unix)]
    #[allow(dead_code)]
    pub fn track_chmod_restore(&mut self, dir: PathBuf) {
        self.restore_writable.push(dir);
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for dir in &self.restore_writable {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
