//! Regression tests covering `FA-SEC01-05`, `FA-SEC01-19`, `FA-SEC01-22`, and `FA-SEC01-23`
//! in `curvebook-platform`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use curvebook_platform::PlatformError;
use curvebook_platform::fs::write_atomic;
use curvebook_platform::sync::{LockError, Mutex, RwLock};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

fn fresh_test_dir(tag: &str) -> std::io::Result<PathBuf> {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = base.join(format!("cb-sec01-{tag}-{}-{seq}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn fa_sec01_05_wait_timeout_honored_when_warn_limit_exceeds_wait_timeout() {
    let wait_timeout = Duration::from_millis(45);
    let warn_limit = Duration::from_millis(600);

    let mu = Arc::new(Mutex::with_limits("fa-sec01-05-mutex", 42_u64, warn_limit, wait_timeout));
    let held_mu = mu.lock().expect("acquire mutex");
    let mu_clone = Arc::clone(&mu);
    let start = Instant::now();
    let err = thread::spawn(move || mu_clone.lock().map(|_| ())).join().expect("thread join").expect_err("must time out");
    let elapsed = start.elapsed();
    assert!(matches!(err, LockError::TimedOut { .. }), "expected TimedOut, got {err:?}");
    assert!(elapsed < Duration::from_millis(300), "mutex slept toward warn_limit ({warn_limit:?}): elapsed={elapsed:?}");
    drop(held_mu);

    let rw = Arc::new(RwLock::with_limits("fa-sec01-05-rwlock", 99_u64, warn_limit, wait_timeout));
    let held_w = rw.write().expect("acquire write");
    let rw_r = Arc::clone(&rw);
    let start_r = Instant::now();
    let err_r = thread::spawn(move || rw_r.read().map(|_| ())).join().expect("thread join").expect_err("read must time out");
    let elapsed_r = start_r.elapsed();
    assert!(matches!(err_r, LockError::TimedOut { .. }), "expected TimedOut, got {err_r:?}");
    assert!(elapsed_r < Duration::from_millis(300), "rwlock read slept toward warn_limit: elapsed={elapsed_r:?}");
    drop(held_w);

    let held_r = rw.read().expect("acquire read");
    let rw_w = Arc::clone(&rw);
    let start_w = Instant::now();
    let err_w = thread::spawn(move || rw_w.write().map(|_| ())).join().expect("thread join").expect_err("write must time out");
    let elapsed_w = start_w.elapsed();
    assert!(matches!(err_w, LockError::TimedOut { .. }), "expected TimedOut, got {err_w:?}");
    assert!(elapsed_w < Duration::from_millis(300), "rwlock write slept toward warn_limit: elapsed={elapsed_w:?}");
    drop(held_r);
}

#[test]
fn fa_sec01_19_waiting_on_registration_cleared_when_contention_callback_panics() {
    let m1 = Arc::new(
        Mutex::with_limits("fa-sec01-19-m1", 1_u32, Duration::from_millis(10), Duration::from_millis(400))
            .with_contention_callback(|_| panic!("intentional callback panic during wait")),
    );
    let m2 = Arc::new(Mutex::with_limits("fa-sec01-19-m2", 2_u32, Duration::from_millis(100), Duration::from_millis(500)));

    let (tx_b_locked, rx_b_locked) = std::sync::mpsc::channel();
    let (tx_b_unwound, rx_b_unwound) = std::sync::mpsc::channel();
    let g1 = m1.lock().expect("A locks m1");

    // Thread A holds m1; thread B holds m2 and tries to lock m1, panicking inside on_contention
    // and catching the unwind while still holding m2. Without RAII Drop cleanup of `waiting_on`,
    // thread B's stale wait edge triggers a false cycle when A locks m2.
    let m1_b = Arc::clone(&m1);
    let m2_b = Arc::clone(&m2);
    let handle_b = thread::spawn(move || {
        let g2 = m2_b.lock().expect("B locks m2");
        tx_b_locked.send(()).expect("notify A");
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m1_b.lock().map(|_| ())));
        assert!(res.is_err(), "contention callback panic should unwind out of lock()");
        tx_b_unwound.send(()).expect("notify unwound");
        thread::sleep(Duration::from_millis(30));
        drop(g2);
    });

    rx_b_locked.recv().expect("wait for B to hold m2");
    rx_b_unwound.recv().expect("wait for B contention panic unwind");
    let m2_lock_res = m2.lock();
    drop(g1);
    handle_b.join().expect("join B");
    assert!(m2_lock_res.is_ok(), "stale waiting_on edge caused false deadlock or error: {:?}", m2_lock_res.err());

    // Same regression check on RwLock write waiting on read holder.
    let r1 = Arc::new(
        RwLock::with_limits("fa-sec01-19-r1", 1_u32, Duration::from_millis(10), Duration::from_millis(400))
            .with_contention_callback(|_| panic!("intentional rwlock callback panic")),
    );
    let r2 = Arc::new(RwLock::with_limits("fa-sec01-19-r2", 2_u32, Duration::from_millis(100), Duration::from_millis(500)));

    let (tx_rb_locked, rx_rb_locked) = std::sync::mpsc::channel();
    let (tx_rb_unwound, rx_rb_unwound) = std::sync::mpsc::channel();
    let gr1 = r1.read().expect("A read-locks r1");
    let r1_b = Arc::clone(&r1);
    let r2_b = Arc::clone(&r2);
    let handle_rb = thread::spawn(move || {
        let g2 = r2_b.write().expect("B locks r2");
        tx_rb_locked.send(()).expect("notify A");
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r1_b.write().map(|_| ())));
        assert!(res.is_err(), "contention callback panic should unwind out of write()");
        tx_rb_unwound.send(()).expect("notify unwound");
        thread::sleep(Duration::from_millis(30));
        drop(g2);
    });

    rx_rb_locked.recv().expect("wait for B to hold r2");
    rx_rb_unwound.recv().expect("wait for B rwlock panic unwind");
    let r2_res = r2.write();
    drop(gr1);
    handle_rb.join().expect("join B");
    assert!(r2_res.is_ok(), "stale RwLock waiting_on edge caused error: {:?}", r2_res.err());
}

#[test]
fn fa_sec01_22_platform_manifest_removes_unused_serde_dep() {
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.contains("serde ="), "unused `serde` dependency must stay removed from platform/Cargo.toml");
}

#[test]
fn fa_sec01_23_write_atomic_rejects_directories_and_special_files() {
    let dir = fresh_test_dir("atomic-special").expect("create temp test dir");
    let sub_dir = dir.join("subdir");
    fs::create_dir_all(&sub_dir).expect("create subdir");

    let dir_err = write_atomic(&sub_dir, b"hello").expect_err("directory target must fail");
    assert!(matches!(dir_err, PlatformError::InvalidPath { .. }), "expected InvalidPath for directory, got {dir_err:?}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let fifo_path = dir.join("named.fifo");
        let status = std::process::Command::new("mkfifo").arg(&fifo_path).status().expect("spawn mkfifo");
        assert!(status.success(), "mkfifo failed");

        let fifo_err = write_atomic(&fifo_path, b"hello").expect_err("FIFO target must be rejected");
        assert!(matches!(fifo_err, PlatformError::InvalidPath { .. }), "expected InvalidPath for FIFO, got {fifo_err:?}");

        let symlink_to_fifo = dir.join("link_to_fifo");
        symlink(&fifo_path, &symlink_to_fifo).expect("create symlink to FIFO");
        let sym_fifo_err = write_atomic(&symlink_to_fifo, b"hello").expect_err("symlink to FIFO must be rejected");
        assert!(matches!(sym_fifo_err, PlatformError::InvalidPath { .. }), "expected InvalidPath for symlink to FIFO, got {sym_fifo_err:?}");

        let symlink_to_dir = dir.join("link_to_dir");
        symlink(&sub_dir, &symlink_to_dir).expect("create symlink to dir");
        let sym_dir_err = write_atomic(&symlink_to_dir, b"hello").expect_err("symlink to dir must be rejected");
        assert!(matches!(sym_dir_err, PlatformError::InvalidPath { .. }), "expected InvalidPath for symlink to dir, got {sym_dir_err:?}");
    }

    let _ = fs::remove_dir_all(&dir);
}
