//! Checked shared-state locks with same-thread re-entry detection, multi-thread wait-for cycle
//! detection, contention reporting, and bounded waits.
//!
//! All production and test shared-state locks across CurveBook use [`Mutex`] and [`RwLock`] rather
//! than raw `std::sync` primitives so re-entrant lock bugs, AB/BA cycles, and wedged holders
//! surface as typed [`LockError`] values instead of hanging or panicking.
//!
//! - **Native targets (`not(target_arch = "wasm32")`):** Tracks per-thread active holders and wait
//!   edges in an internal lock graph, wakes waiters via `Condvar::wait_timeout`, reports
//!   non-long-job holders that keep waiters past `warn_limit`, and returns [`LockError::TimedOut`]
//!   at `wait_timeout` unless an active holder used [`Mutex::lock_long_job`],
//!   [`RwLock::read_long_job`], or [`RwLock::write_long_job`].
//! - **`wasm32-unknown-unknown`:** Maintains single-threaded held-lock checks without
//!   `Instant::now()` or background threads, returning [`LockError::WouldDeadlock`] on incompatible
//!   re-entry and [`LockError::Poisoned`] on poisoned state.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Default wait duration before a non-long-job holder triggers a contention report (`2 s`).
pub const DEFAULT_LOCK_WARN_LIMIT: Duration = Duration::from_secs(2);

/// Default wait duration before a waiter gives up on a non-long-job holder (`10 s`).
pub const DEFAULT_LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

static NEXT_LOCK_ID: AtomicU64 = AtomicU64::new(1);
static TOTAL_CONTENTION_REPORTS: AtomicU64 = AtomicU64::new(0);

/// Return the process-wide count of lock contention reports emitted since startup.
#[must_use]
pub fn total_contention_reports() -> u64 {
    TOTAL_CONTENTION_REPORTS.load(Ordering::Relaxed)
}

/// Configurable warn and timeout durations enforced while waiting on a contended lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockLimits {
    /// Wait duration after which a non-long-job holder emits a contention report.
    pub warn_limit: Duration,
    /// Maximum wait duration before returning [`LockError::TimedOut`] for non-long-job holders.
    pub wait_timeout: Duration,
}

impl Default for LockLimits {
    fn default() -> Self {
        Self { warn_limit: DEFAULT_LOCK_WARN_LIMIT, wait_timeout: DEFAULT_LOCK_WAIT_TIMEOUT }
    }
}

impl LockLimits {
    /// Construct explicit warn and wait-timeout limits.
    #[must_use]
    pub const fn new(warn_limit: Duration, wait_timeout: Duration) -> Self {
        Self { warn_limit, wait_timeout }
    }
}

/// Diagnostic payload emitted once per wait when a non-long-job holder exceeds `warn_limit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentionReport {
    /// Human-readable label of the contended lock.
    pub lock_name: String,
    /// Elapsed wait time in milliseconds when the report was emitted.
    pub waited_ms: u64,
    /// Configured `warn_limit` in milliseconds.
    pub warn_limit_ms: u64,
    /// Label of the waiting thread.
    pub waiter_thread: String,
    /// Labels of threads currently holding the lock.
    pub holder_threads: Vec<String>,
}

impl fmt::Display for ContentionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let holders = if self.holder_threads.is_empty() { String::from("unknown") } else { self.holder_threads.join(", ") };
        write!(
            f,
            "lock `{}` contended for {}ms (warn limit {}ms); waiter={} holders=[{}]",
            self.lock_name, self.waited_ms, self.warn_limit_ms, self.waiter_thread, holders
        )
    }
}

/// Structured error returned when acquiring a [`Mutex`] or [`RwLock`] fails cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// Acquiring the lock would self-reenter incompatibly or close a wait-for cycle.
    WouldDeadlock {
        /// Human-readable label identifying the target lock.
        lock_name: String,
        /// Ordered wait/hold edges forming the deadlock cycle.
        cycle: Vec<String>,
    },
    /// A non-long-job holder kept the lock beyond `wait_timeout`.
    TimedOut {
        /// Human-readable label identifying the target lock.
        lock_name: String,
        /// Total wait time in milliseconds before giving up.
        wait_ms: u64,
    },
    /// A previous writer panicked while holding the underlying synchronization primitive.
    Poisoned {
        /// Human-readable label identifying the poisoned lock.
        lock_name: String,
    },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldDeadlock { lock_name, cycle } => {
                if cycle.is_empty() {
                    write!(f, "lock `{lock_name}` would deadlock")
                } else {
                    write!(f, "lock `{lock_name}` would deadlock: {}", cycle.join(" -> "))
                }
            }
            Self::TimedOut { lock_name, wait_ms } => {
                write!(f, "timed out waiting {wait_ms}ms for lock `{lock_name}`")
            }
            Self::Poisoned { lock_name } => {
                write!(f, "lock `{lock_name}` was poisoned by a panicking holder")
            }
        }
    }
}

impl std::error::Error for LockError {}

/// Structured error returned by non-blocking [`Mutex::try_lock`], [`RwLock::try_read`], and
/// [`RwLock::try_write`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryLockError {
    /// Another thread currently holds an incompatible lock.
    WouldBlock,
    /// Acquiring the lock would self-reenter incompatibly on the current thread.
    WouldDeadlock {
        /// Human-readable label identifying the target lock.
        lock_name: String,
        /// Ordered description of the incompatible self-hold.
        cycle: Vec<String>,
    },
    /// A previous writer panicked while holding the underlying synchronization primitive.
    Poisoned {
        /// Human-readable label identifying the poisoned lock.
        lock_name: String,
    },
}

impl fmt::Display for TryLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldBlock => f.write_str("lock is currently held by another thread"),
            Self::WouldDeadlock { lock_name, cycle } => {
                if cycle.is_empty() {
                    write!(f, "lock `{lock_name}` would deadlock")
                } else {
                    write!(f, "lock `{lock_name}` would deadlock: {}", cycle.join(" -> "))
                }
            }
            Self::Poisoned { lock_name } => {
                write!(f, "lock `{lock_name}` was poisoned by a panicking holder")
            }
        }
    }
}

impl std::error::Error for TryLockError {}

type ContentionCallback = Arc<dyn Fn(&ContentionReport) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LockId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LockMode {
    Shared,
    Exclusive,
}

impl LockMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Exclusive => "exclusive",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AcquireOptions {
    mode: LockMode,
    is_long_job: bool,
    recover_poison: bool,
}

enum RawTryResult<G> {
    Acquired(G),
    WouldBlock,
    Poisoned,
}

#[derive(Debug)]
struct LockRegistrySlot {
    id: LockId,
}

impl Drop for LockRegistrySlot {
    fn drop(&mut self) {
        platform_impl::remove_lock(self.id);
    }
}

struct SyncCore {
    slot: LockRegistrySlot,
    name: String,
    limits: LockLimits,
    contention_reports: AtomicU64,
    on_contention: Option<ContentionCallback>,
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::disallowed_types)]
    wait_mu: std::sync::Mutex<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    wait_cv: std::sync::Condvar,
}

impl SyncCore {
    fn new(name: String, limits: LockLimits) -> Self {
        let id = LockId(NEXT_LOCK_ID.fetch_add(1, Ordering::Relaxed));
        Self {
            slot: LockRegistrySlot { id },
            name,
            limits,
            contention_reports: AtomicU64::new(0),
            on_contention: None,
            #[cfg(not(target_arch = "wasm32"))]
            wait_mu: {
                #[allow(clippy::disallowed_types)]
                std::sync::Mutex::new(0)
            },
            #[cfg(not(target_arch = "wasm32"))]
            wait_cv: std::sync::Condvar::new(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_seq(&self) -> u64 {
        let guard = self.wait_mu.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn bump_seq_and_notify_all(&self) {
        let mut guard = self.wait_mu.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = guard.wrapping_add(1);
        drop(guard);
        self.wait_cv.notify_all();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_for_seq_change(&self, observed_seq: u64, timeout: Duration) {
        let mut guard = self.wait_mu.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *guard != observed_seq {
            return;
        }
        let (next_guard, _wait_res) = self.wait_cv.wait_timeout(guard, timeout).unwrap_or_else(std::sync::PoisonError::into_inner);
        guard = next_guard;
        let _ = *guard;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn emit_contention_report(&self, waited: Duration, waiter_thread: &str, holder_threads: Vec<String>) {
        self.contention_reports.fetch_add(1, Ordering::Relaxed);
        TOTAL_CONTENTION_REPORTS.fetch_add(1, Ordering::Relaxed);
        let report = ContentionReport {
            lock_name: self.name.clone(),
            waited_ms: duration_to_ms_u64(waited),
            warn_limit_ms: duration_to_ms_u64(self.limits.warn_limit),
            waiter_thread: waiter_thread.to_owned(),
            holder_threads,
        };
        if let Some(callback) = &self.on_contention {
            callback(&report);
        } else {
            eprintln!("[curvebook-sync] {report}");
        }
    }

    fn release_hold(&self, is_long_job: bool) {
        platform_impl::release_hold(self.slot.id, is_long_job);
        #[cfg(not(target_arch = "wasm32"))]
        self.bump_seq_and_notify_all();
    }
}

struct GuardReleaseToken<'a> {
    core: &'a SyncCore,
    is_long_job: bool,
}

impl Drop for GuardReleaseToken<'_> {
    fn drop(&mut self) {
        self.core.release_hold(self.is_long_job);
    }
}

/// Mutual-exclusion lock checking same-thread re-entry, multi-thread cycles, and wait budgets.
pub struct Mutex<T: ?Sized> {
    core: SyncCore,
    #[allow(clippy::disallowed_types)]
    inner: std::sync::Mutex<T>,
}

impl<T> Mutex<T> {
    /// Create a new [`Mutex`] using the wrapped type name as its diagnostic lock name.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self::named(core::any::type_name::<T>(), value)
    }

    /// Create a new [`Mutex`] with an explicit diagnostic `name` and default limits.
    #[must_use]
    pub fn named(name: impl Into<String>, value: T) -> Self {
        Self::with_limits(name, value, DEFAULT_LOCK_WARN_LIMIT, DEFAULT_LOCK_WAIT_TIMEOUT)
    }

    /// Create a new [`Mutex`] with explicit `warn_limit` and `wait_timeout` bounds.
    #[must_use]
    pub fn with_limits(name: impl Into<String>, value: T, warn_limit: Duration, wait_timeout: Duration) -> Self {
        Self {
            core: SyncCore::new(name.into(), LockLimits::new(warn_limit, wait_timeout)),
            inner: {
                #[allow(clippy::disallowed_types)]
                std::sync::Mutex::new(value)
            },
        }
    }

    /// Install a callback invoked once per wait when a non-long-job holder exceeds `warn_limit`.
    #[must_use]
    pub fn with_contention_callback(mut self, callback: impl Fn(&ContentionReport) + Send + Sync + 'static) -> Self {
        self.core.on_contention = Some(Arc::new(callback));
        self
    }

    /// Consume the mutex and return its inner value, or [`LockError::Poisoned`] if poisoned.
    ///
    /// # Errors
    /// Returns [`LockError::Poisoned`] if a panicking thread poisoned the underlying mutex.
    pub fn into_inner(self) -> Result<T, LockError> {
        let lock_name = self.core.name.clone();
        self.inner.into_inner().map_err(|_| LockError::Poisoned { lock_name })
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Diagnostic name assigned to this lock.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.core.name
    }

    /// Configured warn and wait-timeout limits for this lock.
    #[must_use]
    pub const fn limits(&self) -> LockLimits {
        self.core.limits
    }

    /// Number of contention reports emitted for this lock instance.
    #[must_use]
    pub fn contention_reports(&self) -> u64 {
        self.core.contention_reports.load(Ordering::Relaxed)
    }

    /// Return `true` if a panicking holder poisoned this mutex.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    /// Clear the poison state so subsequent normal acquisitions succeed again.
    pub fn clear_poison(&self) {
        self.inner.clear_poison();
    }

    /// Acquire the mutex, checking for re-entry, wait-for cycles, poisoning, and wait timeout.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`], [`LockError::TimedOut`], or [`LockError::Poisoned`].
    pub fn lock(&self) -> Result<MutexGuard<'_, T>, LockError> {
        self.acquire_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: false })
    }

    /// Acquire the mutex labelled as an intentional long job so waiters wait past normal limits.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`], [`LockError::TimedOut`], or [`LockError::Poisoned`].
    pub fn lock_long_job(&self) -> Result<MutexGuard<'_, T>, LockError> {
        self.acquire_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: true, recover_poison: false })
    }

    /// Acquire the mutex, recovering inner state if a prior holder panicked while still checking
    /// for deadlocks and timeouts.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`] or [`LockError::TimedOut`].
    pub fn lock_or_recover(&self) -> Result<MutexGuard<'_, T>, LockError> {
        self.acquire_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: true })
    }

    /// Acquire the mutex as a labelled long job, recovering inner state if poisoned.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`] or [`LockError::TimedOut`].
    pub fn lock_long_job_or_recover(&self) -> Result<MutexGuard<'_, T>, LockError> {
        self.acquire_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: true, recover_poison: true })
    }

    /// Attempt to acquire the mutex without blocking.
    ///
    /// # Errors
    /// Returns [`TryLockError::WouldBlock`], [`TryLockError::WouldDeadlock`], or
    /// [`TryLockError::Poisoned`].
    pub fn try_lock(&self) -> Result<MutexGuard<'_, T>, TryLockError> {
        self.try_acquire_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: false })
    }

    /// Attempt to acquire the mutex without blocking, recovering poisoned inner state.
    ///
    /// # Errors
    /// Returns [`TryLockError::WouldBlock`] or [`TryLockError::WouldDeadlock`].
    pub fn try_lock_or_recover(&self) -> Result<MutexGuard<'_, T>, TryLockError> {
        self.try_acquire_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: true })
    }

    /// Borrow the protected data mutably when the caller holds `&mut Mutex<T>`.
    ///
    /// # Errors
    /// Returns [`LockError::Poisoned`] if the underlying mutex is poisoned.
    pub fn get_mut(&mut self) -> Result<&mut T, LockError> {
        let lock_name = self.core.name.clone();
        self.inner.get_mut().map_err(|_| LockError::Poisoned { lock_name })
    }

    fn acquire_with(&self, opts: AcquireOptions) -> Result<MutexGuard<'_, T>, LockError> {
        let guard = platform_impl::acquire(&self.core, opts, || self.raw_try_lock(opts.recover_poison))?;
        Ok(MutexGuard { guard, _release: GuardReleaseToken { core: &self.core, is_long_job: opts.is_long_job } })
    }

    fn try_acquire_with(&self, opts: AcquireOptions) -> Result<MutexGuard<'_, T>, TryLockError> {
        let guard = platform_impl::try_acquire(&self.core, opts, || self.raw_try_lock(opts.recover_poison))?;
        Ok(MutexGuard { guard, _release: GuardReleaseToken { core: &self.core, is_long_job: opts.is_long_job } })
    }

    #[allow(clippy::disallowed_types)]
    fn raw_try_lock(&self, recover_poison: bool) -> RawTryResult<std::sync::MutexGuard<'_, T>> {
        match self.inner.try_lock() {
            Ok(guard) => RawTryResult::Acquired(guard),
            Err(std::sync::TryLockError::WouldBlock) => RawTryResult::WouldBlock,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                if recover_poison {
                    RawTryResult::Acquired(poisoned.into_inner())
                } else {
                    RawTryResult::Poisoned
                }
            }
        }
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> From<T> for Mutex<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Mutex");
        d.field("name", &self.core.name);
        match self.inner.try_lock() {
            Ok(guard) => {
                d.field("data", &&*guard);
            }
            Err(std::sync::TryLockError::Poisoned(err)) => {
                d.field("data", &&**err.get_ref());
                d.field("poisoned", &true);
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                d.field("data", &format_args!("<locked>"));
            }
        }
        d.finish_non_exhaustive()
    }
}

/// RAII exclusive guard returned by [`Mutex`] acquisition methods.
pub struct MutexGuard<'a, T: ?Sized> {
    // Field order is load-bearing: `guard` drops first to unlock the OS mutex before `_release`
    // clears the lock graph and notifies waiting threads.
    #[allow(clippy::disallowed_types)]
    guard: std::sync::MutexGuard<'a, T>,
    _release: GuardReleaseToken<'a>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

/// Reader-writer lock checking same-thread re-entry, multi-thread cycles, and wait budgets.
pub struct RwLock<T: ?Sized> {
    core: SyncCore,
    #[allow(clippy::disallowed_types)]
    inner: std::sync::RwLock<T>,
}

impl<T> RwLock<T> {
    /// Create a new [`RwLock`] using the wrapped type name as its diagnostic lock name.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self::named(core::any::type_name::<T>(), value)
    }

    /// Create a new [`RwLock`] with an explicit diagnostic `name` and default limits.
    #[must_use]
    pub fn named(name: impl Into<String>, value: T) -> Self {
        Self::with_limits(name, value, DEFAULT_LOCK_WARN_LIMIT, DEFAULT_LOCK_WAIT_TIMEOUT)
    }

    /// Create a new [`RwLock`] with explicit `warn_limit` and `wait_timeout` bounds.
    #[must_use]
    pub fn with_limits(name: impl Into<String>, value: T, warn_limit: Duration, wait_timeout: Duration) -> Self {
        Self {
            core: SyncCore::new(name.into(), LockLimits::new(warn_limit, wait_timeout)),
            inner: {
                #[allow(clippy::disallowed_types)]
                std::sync::RwLock::new(value)
            },
        }
    }

    /// Install a callback invoked once per wait when a non-long-job holder exceeds `warn_limit`.
    #[must_use]
    pub fn with_contention_callback(mut self, callback: impl Fn(&ContentionReport) + Send + Sync + 'static) -> Self {
        self.core.on_contention = Some(Arc::new(callback));
        self
    }

    /// Consume the lock and return its inner value, or [`LockError::Poisoned`] if poisoned.
    ///
    /// # Errors
    /// Returns [`LockError::Poisoned`] if a panicking writer poisoned the underlying lock.
    pub fn into_inner(self) -> Result<T, LockError> {
        let lock_name = self.core.name.clone();
        self.inner.into_inner().map_err(|_| LockError::Poisoned { lock_name })
    }
}

impl<T: ?Sized> RwLock<T> {
    /// Diagnostic name assigned to this lock.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.core.name
    }

    /// Configured warn and wait-timeout limits for this lock.
    #[must_use]
    pub const fn limits(&self) -> LockLimits {
        self.core.limits
    }

    /// Number of contention reports emitted for this lock instance.
    #[must_use]
    pub fn contention_reports(&self) -> u64 {
        self.core.contention_reports.load(Ordering::Relaxed)
    }

    /// Return `true` if a panicking writer poisoned this lock.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    /// Clear the poison state so subsequent normal acquisitions succeed again.
    pub fn clear_poison(&self) {
        self.inner.clear_poison();
    }

    /// Acquire shared read access, checking for re-entry, cycles, poisoning, and wait timeout.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`], [`LockError::TimedOut`], or [`LockError::Poisoned`].
    pub fn read(&self) -> Result<RwLockReadGuard<'_, T>, LockError> {
        self.acquire_read_with(AcquireOptions { mode: LockMode::Shared, is_long_job: false, recover_poison: false })
    }

    /// Acquire shared read access labelled as an intentional long job.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`], [`LockError::TimedOut`], or [`LockError::Poisoned`].
    pub fn read_long_job(&self) -> Result<RwLockReadGuard<'_, T>, LockError> {
        self.acquire_read_with(AcquireOptions { mode: LockMode::Shared, is_long_job: true, recover_poison: false })
    }

    /// Acquire shared read access, recovering inner state if poisoned.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`] or [`LockError::TimedOut`].
    pub fn read_or_recover(&self) -> Result<RwLockReadGuard<'_, T>, LockError> {
        self.acquire_read_with(AcquireOptions { mode: LockMode::Shared, is_long_job: false, recover_poison: true })
    }

    /// Acquire shared read access as a labelled long job, recovering inner state if poisoned.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`] or [`LockError::TimedOut`].
    pub fn read_long_job_or_recover(&self) -> Result<RwLockReadGuard<'_, T>, LockError> {
        self.acquire_read_with(AcquireOptions { mode: LockMode::Shared, is_long_job: true, recover_poison: true })
    }

    /// Acquire exclusive write access, checking for re-entry, cycles, poisoning, and wait timeout.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`], [`LockError::TimedOut`], or [`LockError::Poisoned`].
    pub fn write(&self) -> Result<RwLockWriteGuard<'_, T>, LockError> {
        self.acquire_write_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: false })
    }

    /// Acquire exclusive write access labelled as an intentional long job.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`], [`LockError::TimedOut`], or [`LockError::Poisoned`].
    pub fn write_long_job(&self) -> Result<RwLockWriteGuard<'_, T>, LockError> {
        self.acquire_write_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: true, recover_poison: false })
    }

    /// Acquire exclusive write access, recovering inner state if poisoned.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`] or [`LockError::TimedOut`].
    pub fn write_or_recover(&self) -> Result<RwLockWriteGuard<'_, T>, LockError> {
        self.acquire_write_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: true })
    }

    /// Acquire exclusive write access as a labelled long job, recovering inner state if poisoned.
    ///
    /// # Errors
    /// Returns [`LockError::WouldDeadlock`] or [`LockError::TimedOut`].
    pub fn write_long_job_or_recover(&self) -> Result<RwLockWriteGuard<'_, T>, LockError> {
        self.acquire_write_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: true, recover_poison: true })
    }

    /// Attempt to acquire shared read access without blocking.
    ///
    /// # Errors
    /// Returns [`TryLockError::WouldBlock`], [`TryLockError::WouldDeadlock`], or
    /// [`TryLockError::Poisoned`].
    pub fn try_read(&self) -> Result<RwLockReadGuard<'_, T>, TryLockError> {
        self.try_acquire_read_with(AcquireOptions { mode: LockMode::Shared, is_long_job: false, recover_poison: false })
    }

    /// Attempt to acquire shared read access without blocking, recovering poisoned state.
    ///
    /// # Errors
    /// Returns [`TryLockError::WouldBlock`] or [`TryLockError::WouldDeadlock`].
    pub fn try_read_or_recover(&self) -> Result<RwLockReadGuard<'_, T>, TryLockError> {
        self.try_acquire_read_with(AcquireOptions { mode: LockMode::Shared, is_long_job: false, recover_poison: true })
    }

    /// Attempt to acquire exclusive write access without blocking.
    ///
    /// # Errors
    /// Returns [`TryLockError::WouldBlock`], [`TryLockError::WouldDeadlock`], or
    /// [`TryLockError::Poisoned`].
    pub fn try_write(&self) -> Result<RwLockWriteGuard<'_, T>, TryLockError> {
        self.try_acquire_write_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: false })
    }

    /// Attempt to acquire exclusive write access without blocking, recovering poisoned state.
    ///
    /// # Errors
    /// Returns [`TryLockError::WouldBlock`] or [`TryLockError::WouldDeadlock`].
    pub fn try_write_or_recover(&self) -> Result<RwLockWriteGuard<'_, T>, TryLockError> {
        self.try_acquire_write_with(AcquireOptions { mode: LockMode::Exclusive, is_long_job: false, recover_poison: true })
    }

    /// Borrow the protected data mutably when the caller holds `&mut RwLock<T>`.
    ///
    /// # Errors
    /// Returns [`LockError::Poisoned`] if the underlying lock is poisoned.
    pub fn get_mut(&mut self) -> Result<&mut T, LockError> {
        let lock_name = self.core.name.clone();
        self.inner.get_mut().map_err(|_| LockError::Poisoned { lock_name })
    }

    fn acquire_read_with(&self, opts: AcquireOptions) -> Result<RwLockReadGuard<'_, T>, LockError> {
        let guard = platform_impl::acquire(&self.core, opts, || self.raw_try_read(opts.recover_poison))?;
        Ok(RwLockReadGuard { guard, _release: GuardReleaseToken { core: &self.core, is_long_job: opts.is_long_job } })
    }

    fn acquire_write_with(&self, opts: AcquireOptions) -> Result<RwLockWriteGuard<'_, T>, LockError> {
        let guard = platform_impl::acquire(&self.core, opts, || self.raw_try_write(opts.recover_poison))?;
        Ok(RwLockWriteGuard { guard, _release: GuardReleaseToken { core: &self.core, is_long_job: opts.is_long_job } })
    }

    fn try_acquire_read_with(&self, opts: AcquireOptions) -> Result<RwLockReadGuard<'_, T>, TryLockError> {
        let guard = platform_impl::try_acquire(&self.core, opts, || self.raw_try_read(opts.recover_poison))?;
        Ok(RwLockReadGuard { guard, _release: GuardReleaseToken { core: &self.core, is_long_job: opts.is_long_job } })
    }

    fn try_acquire_write_with(&self, opts: AcquireOptions) -> Result<RwLockWriteGuard<'_, T>, TryLockError> {
        let guard = platform_impl::try_acquire(&self.core, opts, || self.raw_try_write(opts.recover_poison))?;
        Ok(RwLockWriteGuard { guard, _release: GuardReleaseToken { core: &self.core, is_long_job: opts.is_long_job } })
    }

    #[allow(clippy::disallowed_types)]
    fn raw_try_read(&self, recover_poison: bool) -> RawTryResult<std::sync::RwLockReadGuard<'_, T>> {
        match self.inner.try_read() {
            Ok(guard) => RawTryResult::Acquired(guard),
            Err(std::sync::TryLockError::WouldBlock) => RawTryResult::WouldBlock,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                if recover_poison {
                    RawTryResult::Acquired(poisoned.into_inner())
                } else {
                    RawTryResult::Poisoned
                }
            }
        }
    }

    #[allow(clippy::disallowed_types)]
    fn raw_try_write(&self, recover_poison: bool) -> RawTryResult<std::sync::RwLockWriteGuard<'_, T>> {
        match self.inner.try_write() {
            Ok(guard) => RawTryResult::Acquired(guard),
            Err(std::sync::TryLockError::WouldBlock) => RawTryResult::WouldBlock,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                if recover_poison {
                    RawTryResult::Acquired(poisoned.into_inner())
                } else {
                    RawTryResult::Poisoned
                }
            }
        }
    }
}

impl<T: Default> Default for RwLock<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> From<T> for RwLock<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("RwLock");
        d.field("name", &self.core.name);
        match self.inner.try_read() {
            Ok(guard) => {
                d.field("data", &&*guard);
            }
            Err(std::sync::TryLockError::Poisoned(err)) => {
                d.field("data", &&**err.get_ref());
                d.field("poisoned", &true);
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                d.field("data", &format_args!("<locked>"));
            }
        }
        d.finish_non_exhaustive()
    }
}

/// RAII shared read guard returned by [`RwLock`] read methods.
pub struct RwLockReadGuard<'a, T: ?Sized> {
    // Field order is load-bearing: `guard` drops before `_release` (see `MutexGuard`).
    #[allow(clippy::disallowed_types)]
    guard: std::sync::RwLockReadGuard<'a, T>,
    _release: GuardReleaseToken<'a>,
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

/// RAII exclusive write guard returned by [`RwLock`] write methods.
pub struct RwLockWriteGuard<'a, T: ?Sized> {
    // Field order is load-bearing: `guard` drops before `_release` (see `MutexGuard`).
    #[allow(clippy::disallowed_types)]
    guard: std::sync::RwLockWriteGuard<'a, T>,
    _release: GuardReleaseToken<'a>,
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn duration_to_ms_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(not(target_arch = "wasm32"))]
mod platform_impl {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::OnceLock;
    use std::thread::ThreadId;
    use std::time::{Duration, Instant};

    use super::{AcquireOptions, LockError, LockId, LockMode, RawTryResult, SyncCore, TryLockError, duration_to_ms_u64};

    #[derive(Debug, Clone)]
    struct HolderRecord {
        thread_id: ThreadId,
        thread_label: String,
        mode: LockMode,
        normal_holds: usize,
        long_job_holds: usize,
    }

    #[derive(Debug, Clone)]
    struct WaitTarget {
        lock_id: LockId,
        lock_name: String,
        mode: LockMode,
        thread_label: String,
    }

    #[derive(Debug, Default)]
    struct LockEntry {
        name: String,
        holders: Vec<HolderRecord>,
    }

    #[derive(Debug, Default)]
    struct LockGraph {
        locks: HashMap<LockId, LockEntry>,
        waiting_on: HashMap<ThreadId, WaitTarget>,
    }

    struct WaitSnapshot {
        holder_is_long_job: bool,
        writer_blocking_shared: bool,
        holder_labels: Vec<String>,
    }

    enum FastAttempt<G> {
        Acquired(G),
        WouldBlock,
        Err(LockError),
    }

    enum BfsNode {
        Lock(LockId, LockMode, usize),
        Thread(ThreadId, usize),
    }

    impl LockGraph {
        fn remove_lock(&mut self, lock_id: LockId) {
            self.locks.remove(&lock_id);
        }

        fn has_waiting_writer(&self, lock_id: LockId) -> bool {
            self.waiting_on.values().any(|w| w.lock_id == lock_id && w.mode == LockMode::Exclusive)
        }

        fn has_long_job_holder(&self, lock_id: LockId) -> bool {
            self.locks.get(&lock_id).is_some_and(|entry| entry.holders.iter().any(|h| h.long_job_holds > 0))
        }

        fn holder_labels(&self, lock_id: LockId) -> Vec<String> {
            self.locks.get(&lock_id).map(|entry| entry.holders.iter().map(|h| h.thread_label.clone()).collect()).unwrap_or_default()
        }

        fn check_self_reentry(
            &self,
            lock_id: LockId,
            lock_name: &str,
            tid: ThreadId,
            thread_label: &str,
            mode: LockMode,
            blocking: bool,
        ) -> Option<LockError> {
            let entry = self.locks.get(&lock_id)?;
            let holder = entry.holders.iter().find(|h| h.thread_id == tid)?;
            let incompatible = match (holder.mode, mode) {
                (LockMode::Exclusive, _) | (LockMode::Shared, LockMode::Exclusive) => true,
                (LockMode::Shared, LockMode::Shared) => blocking || self.has_waiting_writer(lock_id),
            };
            if !incompatible {
                return None;
            }
            let detail = format!("{thread_label} already holds `{lock_name}` ({}) and requested {}", holder.mode.as_str(), mode.as_str());
            Some(LockError::WouldDeadlock { lock_name: lock_name.to_owned(), cycle: vec![detail] })
        }

        fn find_wait_cycle(&self, lock_id: LockId, lock_name: &str, tid: ThreadId, thread_label: &str, mode: LockMode) -> Option<Vec<String>> {
            let mut steps: Vec<(Option<usize>, String)> = vec![(None, format!("{thread_label} wants `{lock_name}` ({})", mode.as_str()))];
            let mut queue = VecDeque::new();
            let mut visited_locks = HashSet::new();
            let mut visited_threads = HashSet::new();

            visited_locks.insert((lock_id, mode));
            queue.push_back(BfsNode::Lock(lock_id, mode, 0));

            while let Some(node) = queue.pop_front() {
                match node {
                    BfsNode::Lock(lid, req_mode, parent_idx) => {
                        if let Some(cycle) = self.expand_lock_neighbors(lid, req_mode, parent_idx, tid, &mut steps, &mut queue, &mut visited_threads)
                        {
                            return Some(cycle);
                        }
                    }
                    BfsNode::Thread(cur_tid, parent_idx) => {
                        if let Some(wt) = self.waiting_on.get(&cur_tid) {
                            let step_idx = steps.len();
                            steps.push((Some(parent_idx), format!("{} waits for `{}` ({})", wt.thread_label, wt.lock_name, wt.mode.as_str())));
                            if visited_locks.insert((wt.lock_id, wt.mode)) {
                                queue.push_back(BfsNode::Lock(wt.lock_id, wt.mode, step_idx));
                            }
                        }
                    }
                }
            }
            None
        }

        #[allow(clippy::too_many_arguments)]
        fn expand_lock_neighbors(
            &self,
            lid: LockId,
            req_mode: LockMode,
            parent_idx: usize,
            requester_tid: ThreadId,
            steps: &mut Vec<(Option<usize>, String)>,
            queue: &mut VecDeque<BfsNode>,
            visited_threads: &mut HashSet<ThreadId>,
        ) -> Option<Vec<String>> {
            if let Some(entry) = self.locks.get(&lid) {
                for holder in &entry.holders {
                    if req_mode == LockMode::Shared && holder.mode == LockMode::Shared {
                        continue;
                    }
                    let step_idx = steps.len();
                    steps.push((Some(parent_idx), format!("`{}` held by {}", entry.name, holder.thread_label)));
                    if holder.thread_id == requester_tid {
                        return Some(reconstruct_cycle(steps, step_idx));
                    }
                    if visited_threads.insert(holder.thread_id) {
                        queue.push_back(BfsNode::Thread(holder.thread_id, step_idx));
                    }
                }
            }
            if req_mode == LockMode::Shared {
                for (wtid, wt) in &self.waiting_on {
                    if wt.lock_id == lid && wt.mode == LockMode::Exclusive {
                        let step_idx = steps.len();
                        steps.push((Some(parent_idx), format!("`{}` has waiting writer {}", wt.lock_name, wt.thread_label)));
                        if *wtid == requester_tid {
                            return Some(reconstruct_cycle(steps, step_idx));
                        }
                        if visited_threads.insert(*wtid) {
                            queue.push_back(BfsNode::Thread(*wtid, step_idx));
                        }
                    }
                }
            }
            None
        }

        fn add_hold(&mut self, lock_id: LockId, lock_name: &str, tid: ThreadId, thread_label: &str, mode: LockMode, is_long_job: bool) {
            let entry = self.locks.entry(lock_id).or_insert_with(|| LockEntry { name: lock_name.to_owned(), holders: Vec::new() });
            if let Some(existing) = entry.holders.iter_mut().find(|h| h.thread_id == tid) {
                if mode == LockMode::Exclusive {
                    existing.mode = LockMode::Exclusive;
                }
                if is_long_job {
                    existing.long_job_holds += 1;
                } else {
                    existing.normal_holds += 1;
                }
                return;
            }
            entry.holders.push(HolderRecord {
                thread_id: tid,
                thread_label: thread_label.to_owned(),
                mode,
                normal_holds: usize::from(!is_long_job),
                long_job_holds: usize::from(is_long_job),
            });
        }

        fn remove_hold(&mut self, lock_id: LockId, tid: ThreadId, is_long_job: bool) {
            let Some(entry) = self.locks.get_mut(&lock_id) else {
                return;
            };
            let Some(pos) = entry.holders.iter().position(|h| h.thread_id == tid) else {
                return;
            };
            let holder = &mut entry.holders[pos];
            if is_long_job {
                holder.long_job_holds = holder.long_job_holds.saturating_sub(1);
            } else {
                holder.normal_holds = holder.normal_holds.saturating_sub(1);
            }
            if holder.normal_holds == 0 && holder.long_job_holds == 0 {
                entry.holders.swap_remove(pos);
            }
        }

        fn cancel_wait(&mut self, tid: ThreadId, lock_id: LockId, mode: LockMode) -> bool {
            let should_remove = self.waiting_on.get(&tid).is_some_and(|w| w.lock_id == lock_id);
            if !should_remove {
                return false;
            }
            self.waiting_on.remove(&tid);
            mode == LockMode::Exclusive
        }
    }

    fn reconstruct_cycle(steps: &[(Option<usize>, String)], tail: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = Some(tail);
        while let Some(idx) = cur {
            let Some((parent, text)) = steps.get(idx) else {
                break;
            };
            out.push(text.clone());
            cur = *parent;
        }
        out.reverse();
        out
    }

    #[allow(clippy::disallowed_types)]
    fn with_graph<R>(f: impl FnOnce(&mut LockGraph) -> R) -> R {
        static GRAPH: OnceLock<std::sync::Mutex<LockGraph>> = OnceLock::new();
        let mu = GRAPH.get_or_init(|| std::sync::Mutex::new(LockGraph::default()));
        let mut guard = mu.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard)
    }

    fn current_thread_identity() -> (ThreadId, String) {
        let current = std::thread::current();
        let tid = current.id();
        let label = match current.name() {
            Some(name) if !name.is_empty() => format!("{name} ({tid:?})"),
            _ => format!("{tid:?}"),
        };
        (tid, label)
    }

    pub(super) fn remove_lock(lock_id: LockId) {
        with_graph(|g| g.remove_lock(lock_id));
    }

    pub(super) fn release_hold(lock_id: LockId, is_long_job: bool) {
        let tid = std::thread::current().id();
        with_graph(|g| g.remove_hold(lock_id, tid, is_long_job));
    }

    pub(super) fn try_acquire<G>(core: &SyncCore, opts: AcquireOptions, mut try_raw: impl FnMut() -> RawTryResult<G>) -> Result<G, TryLockError> {
        let (tid, thread_label) = current_thread_identity();
        with_graph(|g| {
            if let Some(LockError::WouldDeadlock { lock_name, cycle }) =
                g.check_self_reentry(core.slot.id, &core.name, tid, &thread_label, opts.mode, false)
            {
                return Err(TryLockError::WouldDeadlock { lock_name, cycle });
            }
            if opts.mode == LockMode::Shared && g.has_waiting_writer(core.slot.id) {
                return Err(TryLockError::WouldBlock);
            }
            match try_raw() {
                RawTryResult::Acquired(guard) => {
                    g.add_hold(core.slot.id, &core.name, tid, &thread_label, opts.mode, opts.is_long_job);
                    Ok(guard)
                }
                RawTryResult::WouldBlock => Err(TryLockError::WouldBlock),
                RawTryResult::Poisoned => Err(TryLockError::Poisoned { lock_name: core.name.clone() }),
            }
        })
    }

    pub(super) fn acquire<G>(core: &SyncCore, opts: AcquireOptions, mut try_raw: impl FnMut() -> RawTryResult<G>) -> Result<G, LockError> {
        let (tid, thread_label) = current_thread_identity();
        match try_fast_acquire(core, opts, tid, &thread_label, &mut try_raw) {
            FastAttempt::Acquired(guard) => return Ok(guard),
            FastAttempt::Err(err) => return Err(err),
            FastAttempt::WouldBlock => {}
        }

        let wait_started = Instant::now();
        let mut non_long_job_since: Option<Instant> = None;
        let mut warned = false;

        loop {
            let snapshot = prepare_wait_iteration(core, opts, tid, &thread_label, warned)?;
            let observed_seq = core.wait_seq();

            if !snapshot.writer_blocking_shared {
                match try_acquire_from_wait(core, opts, tid, &thread_label, &mut try_raw) {
                    FastAttempt::Acquired(guard) => return Ok(guard),
                    FastAttempt::Err(err) => return Err(err),
                    FastAttempt::WouldBlock => {}
                }
            }

            let now = Instant::now();
            if snapshot.holder_is_long_job {
                non_long_job_since = None;
                let step = core.limits.warn_limit.clamp(Duration::from_millis(5), Duration::from_millis(50));
                core.wait_for_seq_change(observed_seq, step);
                continue;
            }

            let nl_start = *non_long_job_since.get_or_insert(now);
            let elapsed = now.saturating_duration_since(nl_start);
            if !warned && elapsed >= core.limits.warn_limit {
                warned = true;
                core.emit_contention_report(elapsed, &thread_label, snapshot.holder_labels);
            }
            if elapsed >= core.limits.wait_timeout {
                return handle_timeout(core, opts, tid, &thread_label, snapshot.writer_blocking_shared, wait_started, now, &mut try_raw);
            }

            let next_deadline = if warned { core.limits.wait_timeout } else { core.limits.warn_limit };
            let step = next_deadline.saturating_sub(elapsed);
            if !step.is_zero() {
                core.wait_for_seq_change(observed_seq, step);
            }
        }
    }

    fn try_fast_acquire<G>(
        core: &SyncCore,
        opts: AcquireOptions,
        tid: ThreadId,
        thread_label: &str,
        try_raw: &mut impl FnMut() -> RawTryResult<G>,
    ) -> FastAttempt<G> {
        with_graph(|g| {
            if let Some(err) = g.check_self_reentry(core.slot.id, &core.name, tid, thread_label, opts.mode, false) {
                return FastAttempt::Err(err);
            }
            if opts.mode == LockMode::Shared && g.has_waiting_writer(core.slot.id) {
                return FastAttempt::WouldBlock;
            }
            match try_raw() {
                RawTryResult::Acquired(guard) => {
                    g.add_hold(core.slot.id, &core.name, tid, thread_label, opts.mode, opts.is_long_job);
                    FastAttempt::Acquired(guard)
                }
                RawTryResult::WouldBlock => FastAttempt::WouldBlock,
                RawTryResult::Poisoned => FastAttempt::Err(LockError::Poisoned { lock_name: core.name.clone() }),
            }
        })
    }

    fn prepare_wait_iteration(
        core: &SyncCore,
        opts: AcquireOptions,
        tid: ThreadId,
        thread_label: &str,
        warned: bool,
    ) -> Result<WaitSnapshot, LockError> {
        let mut notify_readers = false;
        let result = with_graph(|g| {
            if let Some(err) = g.check_self_reentry(core.slot.id, &core.name, tid, thread_label, opts.mode, true) {
                notify_readers = g.cancel_wait(tid, core.slot.id, opts.mode);
                return Err(err);
            }
            if let Some(cycle) = g.find_wait_cycle(core.slot.id, &core.name, tid, thread_label, opts.mode) {
                notify_readers = g.cancel_wait(tid, core.slot.id, opts.mode);
                return Err(LockError::WouldDeadlock { lock_name: core.name.clone(), cycle });
            }
            g.waiting_on.insert(
                tid,
                WaitTarget { lock_id: core.slot.id, lock_name: core.name.clone(), mode: opts.mode, thread_label: thread_label.to_owned() },
            );
            let holder_is_long_job = g.has_long_job_holder(core.slot.id);
            let writer_blocking_shared = opts.mode == LockMode::Shared && g.has_waiting_writer(core.slot.id);
            let holder_labels = if !holder_is_long_job && !warned { g.holder_labels(core.slot.id) } else { Vec::new() };
            Ok(WaitSnapshot { holder_is_long_job, writer_blocking_shared, holder_labels })
        });
        if notify_readers {
            core.bump_seq_and_notify_all();
        }
        result
    }

    fn try_acquire_from_wait<G>(
        core: &SyncCore,
        opts: AcquireOptions,
        tid: ThreadId,
        thread_label: &str,
        try_raw: &mut impl FnMut() -> RawTryResult<G>,
    ) -> FastAttempt<G> {
        let mut notify_readers = false;
        let attempt = with_graph(|g| match try_raw() {
            RawTryResult::Acquired(guard) => {
                let _ = g.cancel_wait(tid, core.slot.id, opts.mode);
                g.add_hold(core.slot.id, &core.name, tid, thread_label, opts.mode, opts.is_long_job);
                FastAttempt::Acquired(guard)
            }
            RawTryResult::WouldBlock => FastAttempt::WouldBlock,
            RawTryResult::Poisoned => {
                notify_readers = g.cancel_wait(tid, core.slot.id, opts.mode);
                FastAttempt::Err(LockError::Poisoned { lock_name: core.name.clone() })
            }
        });
        if notify_readers {
            core.bump_seq_and_notify_all();
        }
        attempt
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_timeout<G>(
        core: &SyncCore,
        opts: AcquireOptions,
        tid: ThreadId,
        thread_label: &str,
        writer_blocking_shared: bool,
        wait_started: Instant,
        now: Instant,
        try_raw: &mut impl FnMut() -> RawTryResult<G>,
    ) -> Result<G, LockError> {
        if !writer_blocking_shared {
            match try_acquire_from_wait(core, opts, tid, thread_label, try_raw) {
                FastAttempt::Acquired(guard) => return Ok(guard),
                FastAttempt::Err(err) => return Err(err),
                FastAttempt::WouldBlock => {}
            }
        }
        let notify_readers = with_graph(|g| g.cancel_wait(tid, core.slot.id, opts.mode));
        if notify_readers {
            core.bump_seq_and_notify_all();
        }
        Err(LockError::TimedOut { lock_name: core.name.clone(), wait_ms: duration_to_ms_u64(now.saturating_duration_since(wait_started)) })
    }
}

#[cfg(target_arch = "wasm32")]
mod platform_impl {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::{AcquireOptions, LockError, LockId, LockMode, RawTryResult, SyncCore, TryLockError};

    #[derive(Debug, Clone, Copy)]
    struct WasmHold {
        mode: LockMode,
        count: usize,
    }

    thread_local! {
        static WASM_HOLDS: RefCell<HashMap<LockId, WasmHold>> = RefCell::new(HashMap::new());
    }

    pub(super) fn remove_lock(lock_id: LockId) {
        WASM_HOLDS.with(|cell| {
            let _ = cell.borrow_mut().remove(&lock_id);
        });
    }

    pub(super) fn release_hold(lock_id: LockId, _is_long_job: bool) {
        WASM_HOLDS.with(|cell| {
            let mut map = cell.borrow_mut();
            if let Some(hold) = map.get_mut(&lock_id) {
                hold.count = hold.count.saturating_sub(1);
                if hold.count == 0 {
                    let _ = map.remove(&lock_id);
                }
            }
        });
    }

    pub(super) fn try_acquire<G>(core: &SyncCore, opts: AcquireOptions, mut try_raw: impl FnMut() -> RawTryResult<G>) -> Result<G, TryLockError> {
        let conflict = WASM_HOLDS.with(|cell| {
            let map = cell.borrow();
            let hold = map.get(&core.slot.id)?;
            let incompatible = matches!((hold.mode, opts.mode), (LockMode::Exclusive, _) | (LockMode::Shared, LockMode::Exclusive));
            if incompatible { Some(hold.mode) } else { None }
        });
        if let Some(held_mode) = conflict {
            let detail = format!("wasm32 thread already holds `{}` ({}) and requested {}", core.name, held_mode.as_str(), opts.mode.as_str());
            return Err(TryLockError::WouldDeadlock { lock_name: core.name.clone(), cycle: vec![detail] });
        }
        match try_raw() {
            RawTryResult::Acquired(guard) => {
                WASM_HOLDS.with(|cell| {
                    let mut map = cell.borrow_mut();
                    let entry = map.entry(core.slot.id).or_insert(WasmHold { mode: opts.mode, count: 0 });
                    if opts.mode == LockMode::Exclusive {
                        entry.mode = LockMode::Exclusive;
                    }
                    entry.count += 1;
                });
                Ok(guard)
            }
            RawTryResult::WouldBlock => Err(TryLockError::WouldBlock),
            RawTryResult::Poisoned => Err(TryLockError::Poisoned { lock_name: core.name.clone() }),
        }
    }

    pub(super) fn acquire<G>(core: &SyncCore, opts: AcquireOptions, try_raw: impl FnMut() -> RawTryResult<G>) -> Result<G, LockError> {
        try_acquire(core, opts, try_raw).map_err(|err| match err {
            TryLockError::WouldBlock => LockError::WouldDeadlock {
                lock_name: core.name.clone(),
                cycle: vec![format!("wasm32 single-threaded lock `{}` cannot wait on blocked state", core.name)],
            },
            TryLockError::WouldDeadlock { lock_name, cycle } => LockError::WouldDeadlock { lock_name, cycle },
            TryLockError::Poisoned { lock_name } => LockError::Poisoned { lock_name },
        })
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::{Duration, Instant};

    use super::{ContentionReport, LockError, Mutex, RwLock, TryLockError, total_contention_reports};

    #[test]
    fn self_reentry_on_mutex_and_rwlock_returns_would_deadlock() {
        let mu = Mutex::named("test.reentry.mutex", 10_u32);
        {
            let _guard = mu.lock().unwrap();
            let err = mu.lock().unwrap_err();
            assert!(matches!(err, LockError::WouldDeadlock { ref lock_name, ref cycle } if lock_name == "test.reentry.mutex" && !cycle.is_empty()));
            assert!(matches!(mu.try_lock(), Err(TryLockError::WouldDeadlock { .. })));
        }
        assert_eq!(*mu.lock().unwrap(), 10);

        let rw = RwLock::named("test.reentry.rwlock", 20_u32);
        {
            let _w = rw.write().unwrap();
            assert!(matches!(rw.write(), Err(LockError::WouldDeadlock { ref lock_name, .. }) if lock_name == "test.reentry.rwlock"));
            assert!(matches!(rw.read(), Err(LockError::WouldDeadlock { ref lock_name, .. }) if lock_name == "test.reentry.rwlock"));
            assert!(matches!(rw.try_write(), Err(TryLockError::WouldDeadlock { .. })));
            assert!(matches!(rw.try_read(), Err(TryLockError::WouldDeadlock { .. })));
        }
        {
            let _r = rw.read().unwrap();
            assert!(matches!(rw.write(), Err(LockError::WouldDeadlock { ref lock_name, .. }) if lock_name == "test.reentry.rwlock"));
            assert!(matches!(rw.try_write(), Err(TryLockError::WouldDeadlock { .. })));
            // Uncontended compatible shared read succeeds without blocking.
            let r2 = rw.read().unwrap();
            assert_eq!(*r2, 20);
        }
        assert_eq!(*rw.write().unwrap(), 20);
    }

    #[test]
    fn rwlock_reentrant_read_while_writer_waits_returns_would_deadlock() {
        let rw = Arc::new(RwLock::with_limits("test.rw.reentrant_with_writer", 7_u32, Duration::from_secs(2), Duration::from_secs(5)));
        let r1 = rw.read().unwrap();

        let (writer_entered_tx, writer_entered_rx) = mpsc::channel();
        let rw_bg = Arc::clone(&rw);
        let writer_handle = std::thread::spawn(move || {
            writer_entered_tx.send(()).unwrap();
            let mut w = rw_bg.write().unwrap();
            *w = 99;
        });
        writer_entered_rx.recv().unwrap();

        // Wait until the background writer registers its `waiting_on` edge before re-entering `read()`.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_deadlock = false;
        while Instant::now() < deadline {
            match rw.read() {
                Err(LockError::WouldDeadlock { .. }) => {
                    saw_deadlock = true;
                    break;
                }
                Ok(r2) => {
                    drop(r2);
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(other) => panic!("unexpected error waiting for writer registration: {other}"),
            }
        }
        assert!(saw_deadlock, "re-entering read() while a writer waits must return WouldDeadlock");
        drop(r1);
        writer_handle.join().unwrap();
        assert_eq!(*rw.read().unwrap(), 99);
    }

    #[test]
    fn two_thread_ab_ba_mutex_cycle_returns_would_deadlock_without_hanging() {
        let a = Arc::new(Mutex::with_limits("cycle.alpha", 1_u32, Duration::from_secs(2), Duration::from_secs(5)));
        let b = Arc::new(Mutex::with_limits("cycle.beta", 2_u32, Duration::from_secs(2), Duration::from_secs(5)));
        let barrier = Arc::new(Barrier::new(2));

        let a1 = Arc::clone(&a);
        let b1 = Arc::clone(&b);
        let bar1 = Arc::clone(&barrier);
        let t1 = std::thread::spawn(move || {
            let _ga = a1.lock().unwrap();
            bar1.wait();
            b1.lock().map(|gb| *gb)
        });

        let a2 = Arc::clone(&a);
        let b2 = Arc::clone(&b);
        let bar2 = Arc::clone(&barrier);
        let t2 = std::thread::spawn(move || {
            let _gb = b2.lock().unwrap();
            bar2.wait();
            a2.lock().map(|ga| *ga)
        });

        let start = Instant::now();
        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed < Duration::from_secs(2), "cycle detection took too long ({elapsed:?}), likely timed out instead of detecting cycle");
        match (r1, r2) {
            (Ok(2), Err(LockError::WouldDeadlock { lock_name, cycle })) => {
                assert_eq!(lock_name, "cycle.alpha");
                assert!(cycle.len() >= 3, "expected cycle path, got {cycle:?}");
            }
            (Err(LockError::WouldDeadlock { lock_name, cycle }), Ok(1)) => {
                assert_eq!(lock_name, "cycle.beta");
                assert!(cycle.len() >= 3, "expected cycle path, got {cycle:?}");
            }
            other => panic!("expected one thread to succeed after the other returned WouldDeadlock, got {other:?}"),
        }
    }

    #[test]
    fn two_thread_ab_ba_rwlock_cycle_returns_would_deadlock_without_hanging() {
        let a = Arc::new(RwLock::with_limits("cycle.rw.alpha", 10_u32, Duration::from_secs(2), Duration::from_secs(5)));
        let b = Arc::new(RwLock::with_limits("cycle.rw.beta", 20_u32, Duration::from_secs(2), Duration::from_secs(5)));
        let barrier = Arc::new(Barrier::new(2));

        let a1 = Arc::clone(&a);
        let b1 = Arc::clone(&b);
        let bar1 = Arc::clone(&barrier);
        let t1 = std::thread::spawn(move || {
            let _ga = a1.read().unwrap();
            bar1.wait();
            b1.write().map(|mut gb| {
                *gb += 1;
                *gb
            })
        });

        let a2 = Arc::clone(&a);
        let b2 = Arc::clone(&b);
        let bar2 = Arc::clone(&barrier);
        let t2 = std::thread::spawn(move || {
            let _gb = b2.read().unwrap();
            bar2.wait();
            a2.write().map(|mut ga| {
                *ga += 1;
                *ga
            })
        });

        let start = Instant::now();
        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(
            matches!((&r1, &r2), (Ok(_), Err(LockError::WouldDeadlock { .. })) | (Err(LockError::WouldDeadlock { .. }), Ok(_))),
            "expected one Ok and one WouldDeadlock, got ({r1:?}, {r2:?})"
        );
    }

    #[test]
    fn labelled_long_job_is_waited_for_past_normal_limit_without_cutoff() {
        let mu = Arc::new(Mutex::with_limits("test.long_job.mutex", 0_u32, Duration::from_millis(10), Duration::from_millis(35)));
        let rw = Arc::new(RwLock::with_limits("test.long_job.rwlock", 0_u32, Duration::from_millis(10), Duration::from_millis(35)));

        // 1. Mutex long job held past 2.5x normal wait_timeout.
        let waiter_started = Arc::new(AtomicBool::new(false));
        let waiter_started_bg = Arc::clone(&waiter_started);
        let mu_holder = Arc::clone(&mu);
        let (locked_tx, locked_rx) = mpsc::channel();
        let h_mu = std::thread::spawn(move || {
            let mut guard = mu_holder.lock_long_job().unwrap();
            locked_tx.send(()).unwrap();
            while !waiter_started_bg.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(2));
            }
            std::thread::sleep(Duration::from_millis(90));
            *guard = 42;
        });
        locked_rx.recv().unwrap();
        waiter_started.store(true, Ordering::Release);
        let val = *mu.lock().unwrap();
        assert_eq!(val, 42);
        assert_eq!(mu.contention_reports(), 0, "long-job holder must not emit spurious contention reports");
        h_mu.join().unwrap();

        // 2. RwLock write_long_job waited on by read().
        let rw_waiter_started = Arc::new(AtomicBool::new(false));
        let rw_waiter_started_bg = Arc::clone(&rw_waiter_started);
        let rw_holder = Arc::clone(&rw);
        let (rw_locked_tx, rw_locked_rx) = mpsc::channel();
        let h_rw = std::thread::spawn(move || {
            let mut guard = rw_holder.write_long_job().unwrap();
            rw_locked_tx.send(()).unwrap();
            while !rw_waiter_started_bg.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(2));
            }
            std::thread::sleep(Duration::from_millis(90));
            *guard = 77;
        });
        rw_locked_rx.recv().unwrap();
        rw_waiter_started.store(true, Ordering::Release);
        assert_eq!(*rw.read().unwrap(), 77);
        assert_eq!(rw.contention_reports(), 0);
        h_rw.join().unwrap();
    }

    #[test]
    fn poisoning_detection_and_recovery_on_mutex_and_rwlock() {
        let mu = Arc::new(Mutex::named("test.poison.mutex", 5_u32));
        let mu_bg = Arc::clone(&mu);
        let join = std::thread::spawn(move || {
            let mut g = mu_bg.lock().unwrap();
            *g = 11;
            panic!("simulated panic while holding mutex");
        });
        assert!(join.join().is_err());
        assert!(mu.is_poisoned());
        assert!(matches!(mu.lock(), Err(LockError::Poisoned { ref lock_name }) if lock_name == "test.poison.mutex"));
        assert!(matches!(mu.try_lock(), Err(TryLockError::Poisoned { ref lock_name }) if lock_name == "test.poison.mutex"));
        {
            let mut recovered = mu.lock_or_recover().unwrap();
            assert_eq!(*recovered, 11);
            *recovered = 12;
        }
        mu.clear_poison();
        assert!(!mu.is_poisoned());
        assert_eq!(*mu.lock().unwrap(), 12);

        let rw = Arc::new(RwLock::named("test.poison.rwlock", 100_u32));
        let rw_bg = Arc::clone(&rw);
        let join_rw = std::thread::spawn(move || {
            let mut g = rw_bg.write().unwrap();
            *g = 200;
            panic!("simulated panic while holding rwlock write");
        });
        assert!(join_rw.join().is_err());
        assert!(rw.is_poisoned());
        assert!(matches!(rw.read(), Err(LockError::Poisoned { ref lock_name }) if lock_name == "test.poison.rwlock"));
        assert!(matches!(rw.write(), Err(LockError::Poisoned { ref lock_name }) if lock_name == "test.poison.rwlock"));
        assert_eq!(*rw.read_or_recover().unwrap(), 200);
        {
            let mut recovered_w = rw.write_or_recover().unwrap();
            *recovered_w = 300;
        }
        rw.clear_poison();
        assert!(!rw.is_poisoned());
        assert_eq!(*rw.read().unwrap(), 300);
    }

    #[test]
    fn warn_limit_emits_single_contention_report_before_wait_timeout() {
        let reports: Arc<Mutex<Vec<ContentionReport>>> = Arc::new(Mutex::named("test.captured_reports", Vec::new()));
        let reports_cb = Arc::clone(&reports);
        let (warned_tx, warned_rx) = mpsc::channel();
        let warned_tx = Arc::new(warned_tx);

        // Large wait_timeout (2s) so holder releases deterministically as soon as warn_limit (15ms) triggers.
        let mu =
            Arc::new(Mutex::with_limits("test.limits.warn_only", 1_u32, Duration::from_millis(15), Duration::from_secs(2)).with_contention_callback(
                move |rep| {
                    if let Ok(mut guard) = reports_cb.lock() {
                        guard.push(rep.clone());
                    }
                    let _ = warned_tx.send(());
                },
            ));

        let total_before = total_contention_reports();
        let (locked_tx, locked_rx) = mpsc::channel();
        let mu_bg = Arc::clone(&mu);
        let holder = std::thread::Builder::new()
            .name(String::from("warn-holder"))
            .spawn(move || {
                let mut g = mu_bg.lock().unwrap();
                locked_tx.send(()).unwrap();
                warned_rx.recv().unwrap();
                *g = 9;
            })
            .unwrap();

        locked_rx.recv().unwrap();
        assert_eq!(*mu.lock().unwrap(), 9);
        holder.join().unwrap();

        assert_eq!(mu.contention_reports(), 1);
        assert!(total_contention_reports() > total_before);
        let captured = reports.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].lock_name, "test.limits.warn_only");
        assert!(captured[0].holder_threads.iter().any(|h| h.contains("warn-holder")));
    }

    #[test]
    fn wait_timeout_fails_with_timed_out_after_single_contention_report() {
        let report_count = Arc::new(AtomicUsize::new(0));
        let report_count_cb = Arc::clone(&report_count);
        let mu = Arc::new(
            Mutex::with_limits("test.limits.timeout_mutex", 0_u32, Duration::from_millis(15), Duration::from_millis(50)).with_contention_callback(
                move |_| {
                    report_count_cb.fetch_add(1, Ordering::Relaxed);
                },
            ),
        );

        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mu_bg = Arc::clone(&mu);
        let holder = std::thread::spawn(move || {
            let mut g = mu_bg.lock().unwrap();
            locked_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            *g = 55;
        });

        locked_rx.recv().unwrap();
        let err = mu.lock().unwrap_err();
        assert!(matches!(err, LockError::TimedOut { ref lock_name, wait_ms } if lock_name == "test.limits.timeout_mutex" && wait_ms >= 40));
        assert_eq!(mu.contention_reports(), 1, "exactly one contention report per timed-out wait");
        assert_eq!(report_count.load(Ordering::Relaxed), 1);

        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(*mu.lock().unwrap(), 55);

        // Same limits contract on RwLock write-held / read-waiter.
        let rw_reports = Arc::new(AtomicUsize::new(0));
        let rw_reports_cb = Arc::clone(&rw_reports);
        let rw = Arc::new(
            RwLock::with_limits("test.limits.timeout_rwlock", 0_u32, Duration::from_millis(15), Duration::from_millis(50)).with_contention_callback(
                move |_| {
                    rw_reports_cb.fetch_add(1, Ordering::Relaxed);
                },
            ),
        );
        let (rw_locked_tx, rw_locked_rx) = mpsc::channel();
        let (rw_release_tx, rw_release_rx) = mpsc::channel();
        let rw_bg = Arc::clone(&rw);
        let rw_holder = std::thread::spawn(move || {
            let mut g = rw_bg.write().unwrap();
            rw_locked_tx.send(()).unwrap();
            rw_release_rx.recv().unwrap();
            *g = 88;
        });
        rw_locked_rx.recv().unwrap();
        let rw_err = rw.read().unwrap_err();
        assert!(matches!(rw_err, LockError::TimedOut { ref lock_name, wait_ms } if lock_name == "test.limits.timeout_rwlock" && wait_ms >= 40));
        assert_eq!(rw.contention_reports(), 1);
        assert_eq!(rw_reports.load(Ordering::Relaxed), 1);
        rw_release_tx.send(()).unwrap();
        rw_holder.join().unwrap();
        assert_eq!(*rw.read().unwrap(), 88);
    }

    #[test]
    fn concurrent_readers_debug_formatting_and_owned_access_work_cleanly() {
        let rw = Arc::new(RwLock::named("test.rw.concurrent", 123_u32));
        let r1 = rw.read().unwrap();
        let dbg_locked = format!("{rw:?}");
        assert!(dbg_locked.contains("test.rw.concurrent"));
        assert!(dbg_locked.contains("123"));

        let rw_bg = Arc::clone(&rw);
        let r2_val = std::thread::spawn(move || *rw_bg.read().unwrap()).join().unwrap();
        assert_eq!(r2_val, 123);
        drop(r1);

        let mut mu = Mutex::from(String::from("hello"));
        {
            let _g = mu.lock().unwrap();
            let dbg_mu = format!("{mu:?}");
            assert!(dbg_mu.contains("<locked>"));
        }
        mu.get_mut().unwrap().push_str(" world");
        assert_eq!(mu.into_inner().unwrap(), "hello world");

        let err_deadlock = LockError::WouldDeadlock { lock_name: String::from("a"), cycle: vec![String::from("t1 -> a"), String::from("a -> t1")] };
        assert!(err_deadlock.to_string().contains("t1 -> a -> a -> t1"));
        let err_timeout = LockError::TimedOut { lock_name: String::from("b"), wait_ms: 42 };
        assert!(err_timeout.to_string().contains("42ms"));
        let err_poison = LockError::Poisoned { lock_name: String::from("c") };
        assert!(err_poison.to_string().contains("poisoned"));
        assert!(TryLockError::WouldBlock.to_string().contains("another thread"));
    }
}
