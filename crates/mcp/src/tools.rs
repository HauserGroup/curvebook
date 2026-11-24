//! MCP tool schemas, server state management, live desktop `--control` attach client,
//! command dispatch with `Caller::Mcp` + `run_guarded`, and RFC 4648 base64 encoding.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use curvebook_doc::{ErrorCode, ProjectDocument, ResultSheet, SheetId, SheetSection};
use curvebook_engine::cmd::{Caller, CommandGroup, Effects, registry};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_engine::services::command_step_writes_filesystem;
use curvebook_engine::services::{BatchCommandStep, inspect_project_summary, inspect_sheet_detail};
use curvebook_engine::{CommandOutcome, EngineError, ExternalPathPolicy, GuiOnlyPolicy, Session};
use curvebook_platform::sync::{LockError, RwLock, RwLockReadGuard, RwLockWriteGuard};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_platform::{PlatformError, WorkdirGuard};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::errors::{self, StructuredErrorPayload};
use crate::formats;
use crate::gui_tools;
use crate::toolsets::{self, ToolAnnotations, command_id_to_mcp_tool_name, command_spec_to_tool_definition, group_enum_json, parse_command_group};
use crate::ui_catalog::UI_COMMANDS;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use live_control::MAX_CONTROL_RESPONSE_BYTES;

const MAX_BATCH_COMMANDS: usize = 1024;
const MAX_PRE_CANCELLED_IDS: usize = 256;

#[derive(Default)]
struct CancellationInner {
    active: HashMap<String, Arc<AtomicBool>>,
    pre_cancelled: VecDeque<String>,
}

/// Thread-safe registry mapping JSON-RPC request IDs to cooperative [`AtomicBool`] cancellation tokens.
#[derive(Clone)]
pub struct CancellationRegistry {
    inner: Arc<RwLock<CancellationInner>>,
}

impl Default for CancellationRegistry {
    fn default() -> Self {
        Self { inner: Arc::new(RwLock::named("mcp.cancellation_registry", CancellationInner::default())) }
    }
}

impl fmt::Debug for CancellationRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.read_inner() {
            Ok(guard) => f
                .debug_struct("CancellationRegistry")
                .field("active_count", &guard.active.len())
                .field("pre_cancelled_count", &guard.pre_cancelled.len())
                .finish(),
            Err(err) => f.debug_struct("CancellationRegistry").field("lock_error", &err.to_string()).finish(),
        }
    }
}

pub(crate) struct ActiveRequestCancelGuard {
    registry: CancellationRegistry,
    key: String,
}

impl Drop for ActiveRequestCancelGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.registry.write_inner() {
            guard.active.remove(&self.key);
        }
    }
}

fn canonical_request_id_key(id: &Value) -> Option<String> {
    match id {
        Value::String(s) if !s.is_empty() => Some(format!("s:{s}")),
        Value::Number(n) => Some(format!("n:{n}")),
        _ => None,
    }
}

impl CancellationRegistry {
    /// Construct an empty [`CancellationRegistry`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn read_inner(&self) -> Result<RwLockReadGuard<'_, CancellationInner>, LockError> {
        self.inner.read_or_recover()
    }

    fn write_inner(&self) -> Result<RwLockWriteGuard<'_, CancellationInner>, LockError> {
        self.inner.write_or_recover()
    }

    /// Signal cooperative cancellation for `request_id`.
    ///
    /// Returns `true` if an actively executing request matched `request_id` and had its token set.
    /// If no active request currently matches a valid string/number `request_id`, records a bounded
    /// pre-cancellation marker so an immediately queued request (e.g. later in a JSON-RPC batch)
    /// starts pre-cancelled.
    pub fn cancel(&self, request_id: &Value) -> bool {
        let Some(key) = canonical_request_id_key(request_id) else {
            return false;
        };
        let Ok(mut guard) = self.write_inner() else {
            return false;
        };
        if let Some(token) = guard.active.get(&key) {
            token.store(true, Ordering::Release);
            return true;
        }
        if !guard.pre_cancelled.iter().any(|existing| existing == &key) {
            if guard.pre_cancelled.len() >= MAX_PRE_CANCELLED_IDS {
                let _ = guard.pre_cancelled.pop_front();
            }
            guard.pre_cancelled.push_back(key);
        }
        false
    }

    /// Number of currently registered in-flight requests.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.read_inner().map_or(0, |guard| guard.active.len())
    }

    pub(crate) fn begin_request(&self, request_id: &Value) -> (Arc<AtomicBool>, Option<ActiveRequestCancelGuard>) {
        let Some(key) = canonical_request_id_key(request_id) else {
            return (Arc::new(AtomicBool::new(false)), None);
        };
        let Ok(mut guard) = self.write_inner() else {
            return (Arc::new(AtomicBool::new(false)), None);
        };
        let was_pre_cancelled = if let Some(pos) = guard.pre_cancelled.iter().position(|item| item == &key) {
            let _ = guard.pre_cancelled.remove(pos);
            true
        } else {
            false
        };
        let token = Arc::new(AtomicBool::new(was_pre_cancelled));
        guard.active.insert(key.clone(), Arc::clone(&token));
        drop(guard);
        (token, Some(ActiveRequestCancelGuard { registry: self.clone(), key }))
    }
}

/// Host-supplied in-process GUI bridge implemented outside `crates/mcp` (`L6`/`L7`) so the 8
/// `gui` MCP tools can inspect and drive a live UI without `crates/mcp` depending on `ui-egui`.
pub trait GuiSessionHost {
    /// Execute a GUI control method (`ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, etc.).
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError>;

    /// Synchronize `--gui-only` restriction policy onto the hosted UI session.
    fn set_gui_only(&mut self, gui_only: bool);

    /// Return `true` when the hosted UI session enforces `--gui-only` mode.
    fn is_gui_only(&self) -> bool;

    /// Optional shared backing [`Session`] when running alongside an in-process GUI app so core
    /// tools and MCP `resources/*` read and mutate the same document (`R2-S12-07`).
    fn hosted_session(&self) -> Option<&Session> {
        None
    }

    /// Mutably borrow the shared backing [`Session`] for core engine/file operations (`R2-S12-07`).
    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        None
    }

    /// Synchronize UI view state after direct mutations through [`Self::hosted_session_mut`].
    fn sync_after_session_mutation(&mut self) {}
}

#[cfg(not(target_arch = "wasm32"))]
type LiveControlHandler = Box<dyn FnMut(&str, Value) -> Result<Value, String> + Send>;

/// Stateful execution context shared across requests on an MCP server connection.
pub struct McpServerState {
    pub(crate) session: Session,
    #[cfg(not(target_arch = "wasm32"))]
    control_port: Option<u16>,
    #[cfg(not(target_arch = "wasm32"))]
    control_token: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    control_handler: Option<LiveControlHandler>,
    #[cfg(not(target_arch = "wasm32"))]
    control_requested_gui_only: bool,
    #[cfg(not(target_arch = "wasm32"))]
    control_policy_synced: bool,
    #[cfg(not(target_arch = "wasm32"))]
    workdir_guard: Option<WorkdirGuard>,
    gui_host: Option<Box<dyn GuiSessionHost>>,
    gui_only: bool,
    gui_toolset_enabled: bool,
    enabled_toolsets: BTreeSet<CommandGroup>,
    pending_notifications: Vec<Value>,
    initialized: bool,
    require_initialize: bool,
    shutting_down: bool,
    cancellation: CancellationRegistry,
    active_cancel_token: Option<Arc<AtomicBool>>,
}

impl fmt::Debug for McpServerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("McpServerState");
        s.field("session", &self.session);
        #[cfg(not(target_arch = "wasm32"))]
        s.field("control_port", &self.control_port)
            .field("has_control_token", &self.control_token.is_some())
            .field("has_control_handler", &self.control_handler.is_some())
            .field("control_requested_gui_only", &self.control_requested_gui_only)
            .field("control_policy_synced", &self.control_policy_synced)
            .field("workdir_guard", &self.workdir_guard);
        s.field("has_gui_host", &self.gui_host.is_some())
            .field("gui_only", &self.gui_only)
            .field("gui_toolset_enabled", &self.gui_toolset_enabled)
            .field("enabled_toolsets", &self.enabled_toolsets)
            .field("pending_notifications_len", &self.pending_notifications.len())
            .field("initialized", &self.initialized)
            .field("require_initialize", &self.require_initialize)
            .field("shutting_down", &self.shutting_down)
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

impl Default for McpServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServerState {
    /// Construct a fresh headless MCP server state backed by an empty [`Session`].
    #[must_use]
    pub fn new() -> Self {
        Self::from_session(Session::new())
    }

    /// Wrap an existing headless [`Session`].
    #[must_use]
    pub fn from_session(session: Session) -> Self {
        let gui_only = session.gui_only() == GuiOnlyPolicy::Strict;
        #[cfg(not(target_arch = "wasm32"))]
        let workdir_guard = session.path_policy().workdir_guard().cloned();
        Self {
            session,
            #[cfg(not(target_arch = "wasm32"))]
            control_port: None,
            #[cfg(not(target_arch = "wasm32"))]
            control_token: None,
            #[cfg(not(target_arch = "wasm32"))]
            control_handler: None,
            #[cfg(not(target_arch = "wasm32"))]
            control_requested_gui_only: false,
            #[cfg(not(target_arch = "wasm32"))]
            control_policy_synced: false,
            #[cfg(not(target_arch = "wasm32"))]
            workdir_guard,
            gui_host: None,
            gui_only,
            gui_toolset_enabled: false,
            enabled_toolsets: BTreeSet::new(),
            pending_notifications: Vec::new(),
            initialized: false,
            require_initialize: false,
            shutting_down: false,
            cancellation: CancellationRegistry::new(),
            active_cancel_token: None,
        }
    }

    /// Construct an MCP server state attached to a running desktop window listening on
    /// `127.0.0.1:<port>`.
    ///
    /// Live-attached server states cannot subsequently enable `--workdir` confinement because
    /// `--workdir` cannot confine file operations inside the external desktop host process.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_control_port(port: u16) -> Result<Self, String> {
        if port == 0 {
            return Err(String::from("control port must be in 1..=65535"));
        }
        let mut state = Self::new();
        state.control_port = Some(port);
        Ok(state)
    }

    /// Construct an MCP server state attached to a running desktop `--control <port>` with explicit
    /// `--gui-only` policy state.
    ///
    /// Live-attached server states cannot subsequently enable `--workdir` confinement because
    /// `--workdir` cannot confine file operations inside the external desktop host process.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_control_port_and_gui_only(port: u16, gui_only: bool) -> Result<Self, String> {
        let mut state = Self::with_control_port(port)?;
        state.control_requested_gui_only = gui_only;
        state.apply_local_gui_only(gui_only);
        Ok(state)
    }

    /// Attach an in-process JSON-lines control dispatcher for sandboxed integration tests.
    #[doc(hidden)]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_control_handler<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&str, Value) -> Result<Value, String> + Send + 'static,
    {
        self.control_handler = Some(Box::new(handler));
        self.control_policy_synced = false;
        self
    }

    /// Construct a headless MCP server state whose file operations are confined strictly inside `workdir`.
    ///
    /// Confined server states cannot subsequently attach a live `control_port` because `--workdir`
    /// cannot confine file operations inside an external desktop host process.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_workdir(workdir: &Path) -> Result<Self, PlatformError> {
        let guard = WorkdirGuard::new(workdir)?;
        let session = Session::new().with_path_policy(ExternalPathPolicy::from_guard(guard));
        Ok(Self::from_session(session))
    }

    /// Construct an MCP server state backed by an in-process [`GuiSessionHost`].
    #[must_use]
    pub fn with_gui_host(host: Box<dyn GuiSessionHost>) -> Self {
        let gui_only = host.is_gui_only();
        let mut session = Session::new();
        session.set_gui_only(if gui_only { GuiOnlyPolicy::Strict } else { GuiOnlyPolicy::Disabled });
        let mut state = Self::from_session(session);
        state.gui_host = Some(host);
        state.gui_only = gui_only;
        state.sync_policies_to_hosted_session();
        state
    }

    /// Fluent builder to enable or disable strict `--gui-only` mode on this server state.
    #[must_use]
    pub fn with_gui_only(mut self, gui_only: bool) -> Self {
        self.set_gui_only(gui_only);
        self
    }

    /// Attach or clear a canonical [`WorkdirGuard`] confinement root on this server state.
    ///
    /// Returns [`PlatformError::InvalidPath`] when a live desktop `control_port` is configured,
    /// because `--workdir` cannot confine file operations executed inside a live-attached desktop
    /// host process.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_workdir(&mut self, workdir: Option<&Path>) -> Result<(), PlatformError> {
        if workdir.is_some() && self.control_port.is_some() {
            return Err(PlatformError::InvalidPath {
                reason: String::from(
                    "cannot combine --workdir with --control-port: `--workdir` cannot confine file operations on a live-attached desktop host",
                ),
            });
        }
        let policy = match workdir {
            Some(dir) => {
                let guard = WorkdirGuard::new(dir)?;
                self.workdir_guard = Some(guard.clone());
                ExternalPathPolicy::from_guard(guard)
            }
            None => {
                self.workdir_guard = None;
                ExternalPathPolicy::Unconstrained
            }
        };
        self.session.set_path_policy(policy.clone());
        if let Some(host) = self.gui_host.as_mut()
            && let Some(hosted) = host.hosted_session_mut()
        {
            hosted.set_path_policy(policy);
        }
        Ok(())
    }

    fn effective_path_policy(&self) -> ExternalPathPolicy {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(guard) = self.workdir_guard.as_ref() {
            return ExternalPathPolicy::from_guard(guard.clone());
        }
        if *self.session.path_policy() == ExternalPathPolicy::RejectExternal {
            return ExternalPathPolicy::RejectExternal;
        }
        ExternalPathPolicy::Unconstrained
    }

    fn sync_policies_to_hosted_session(&mut self) {
        let gui_policy = if self.gui_only { GuiOnlyPolicy::Strict } else { GuiOnlyPolicy::Disabled };
        let path_policy = self.effective_path_policy();
        if let Some(host) = self.gui_host.as_mut()
            && let Some(hosted) = host.hosted_session_mut()
        {
            hosted.set_gui_only(gui_policy);
            if *hosted.path_policy() != path_policy && path_policy != ExternalPathPolicy::Unconstrained {
                hosted.set_path_policy(path_policy);
            }
        }
    }

    /// Execute `f` against the active [`Session`] (the attached `GuiSessionHost` session when
    /// available, otherwise `self.session`), and notify the GUI host after mutation.
    pub(crate) fn mutate_active_session<R>(&mut self, f: impl FnOnce(&mut Session) -> R) -> R {
        if let Some(host) = self.gui_host.as_mut()
            && host.hosted_session().is_some()
            && let Some(hosted) = host.hosted_session_mut()
        {
            let res = f(hosted);
            host.sync_after_session_mutation();
            return res;
        }
        f(&mut self.session)
    }

    /// Replace the active [`Session`] (or hosted `GuiSessionHost` session) while preserving
    /// configured `--gui-only` and `--workdir` policies across `open_file` reloads and rollbacks.
    pub(crate) fn install_session(&mut self, mut session: Session) {
        let gui_policy = if self.gui_only { GuiOnlyPolicy::Strict } else { GuiOnlyPolicy::Disabled };
        session.set_gui_only(gui_policy);
        let current_policy = self.effective_path_policy();
        if current_policy != ExternalPathPolicy::Unconstrained {
            session.set_path_policy(current_policy);
        }
        if let Some(host) = self.gui_host.as_mut()
            && let Some(hosted) = host.hosted_session_mut()
        {
            *hosted = session;
            host.sync_after_session_mutation();
            return;
        }
        self.session = session;
    }

    /// Active [`WorkdirGuard`] confinement root, when configured.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn workdir_guard(&self) -> Option<&WorkdirGuard> {
        self.workdir_guard.as_ref()
    }

    /// Borrow the active backing [`Session`] (the attached `GuiSessionHost` session when hosted,
    /// otherwise this state's standalone session).
    #[must_use]
    pub fn session(&self) -> &Session {
        if let Some(hosted) = self.gui_host.as_deref().and_then(GuiSessionHost::hosted_session) {
            return hosted;
        }
        &self.session
    }

    /// Mutably borrow the active backing [`Session`].
    pub fn session_mut(&mut self) -> &mut Session {
        let has_hosted = self.gui_host.as_deref().and_then(GuiSessionHost::hosted_session).is_some();
        if has_hosted
            && let Some(host) = self.gui_host.as_mut()
            && let Some(hosted) = host.hosted_session_mut()
        {
            return hosted;
        }
        &mut self.session
    }

    /// Configured live desktop control port on `127.0.0.1`, if attached.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn control_port(&self) -> Option<u16> {
        self.control_port
    }

    /// Live desktop attach is unavailable on `wasm32`.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn control_port(&self) -> Option<u16> {
        None
    }

    /// Attach or detach a live desktop control port on `127.0.0.1`.
    ///
    /// Rejects port `0` and rejects attaching a live `control_port` when `--workdir` confinement
    /// is configured, because `--workdir` cannot confine file operations executed inside a
    /// live-attached desktop host process.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_control_port(&mut self, port: Option<u16>) -> Result<(), String> {
        if port == Some(0) {
            return Err(String::from("control port must be in 1..=65535"));
        }
        if port.is_some() && (self.workdir_guard.is_some() || self.session().path_policy().workdir_guard().is_some()) {
            return Err(String::from(
                "cannot combine --control-port with --workdir: `--workdir` cannot confine file operations on a live-attached desktop host",
            ));
        }
        self.control_port = port;
        self.control_policy_synced = false;
        Ok(())
    }

    /// Explicit authentication token configured for live `--control` connections, if set.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn control_token(&self) -> Option<&str> {
        self.control_token.as_deref()
    }

    /// Fluent builder to attach an explicit `ControlAuth` token for live `--control` requests.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_control_token(mut self, token: impl AsRef<str>) -> Result<Self, String> {
        self.set_control_token(Some(token.as_ref()))?;
        Ok(self)
    }

    /// Attach or clear an explicit `ControlAuth` token for live `--control` requests.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_control_token(&mut self, token: Option<&str>) -> Result<(), String> {
        self.control_token = token.map(live_control::validate_control_token).transpose()?;
        self.control_policy_synced = false;
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn resolved_control_token(&self, port: u16) -> Result<Option<String>, StructuredErrorPayload> {
        if let Some(ref explicit) = self.control_token {
            return Ok(Some(explicit.clone()));
        }
        live_control::resolve_default_control_token(port)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn send_live_control_raw(&mut self, method: &str, params: Value) -> Result<Value, StructuredErrorPayload> {
        let Some(port) = self.control_port() else {
            return Err(StructuredErrorPayload::new(ErrorCode::Conflict, "live control request attempted without an attached --control-port"));
        };
        if let Some(ref mut handler) = self.control_handler {
            return (handler)(method, params).map_err(|msg| {
                let lower = msg.to_ascii_lowercase();
                let code = if lower.contains("not found") {
                    ErrorCode::NotFound
                } else if lower.contains("cancelled") {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::BadInput
                };
                StructuredErrorPayload::new(code, msg)
            });
        }
        let token = self.resolved_control_token(port)?;
        live_control::send_control_detailed_with_token(port, token.as_deref(), method, params)
    }

    /// Synchronize `--gui-only` policy with an attached `--control-port` host via `ping`,
    /// failing closed when the handshake fails or when local `--gui-only` cannot be enforced on an
    /// unlocked remote host (`R2-S12-01`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn sync_remote_control_policy(&mut self) -> Result<(), StructuredErrorPayload> {
        if self.control_port.is_none() || self.control_policy_synced {
            return Ok(());
        }
        let ping = self.send_live_control_raw("ping", json!({}))?;
        let Some(remote_gui_only) = ping.get("gui_only").and_then(Value::as_bool) else {
            return Err(StructuredErrorPayload::new(ErrorCode::Conflict, "remote control host `ping` response omitted boolean `gui_only` policy")
                .with_remediation("Attach only to a CurveBook control server returning `gui_only` from `ping`."));
        };
        if self.control_requested_gui_only && !remote_gui_only {
            return Err(StructuredErrorPayload::new(
                ErrorCode::GuiOnlyViolation,
                "cannot enforce `--gui-only` over `--control-port`: remote desktop host does not have `--gui-only` enabled",
            )
            .with_remediation("Restart the remote desktop host with `--gui-only` before attaching `curvebook-cli mcp --control-port ... --gui-only`.")
            .with_details(json!({ "local_gui_only": true, "remote_gui_only": false })));
        }
        self.apply_local_gui_only(remote_gui_only);
        self.control_policy_synced = true;
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_remote_control_policy(&mut self) -> Result<(), StructuredErrorPayload> {
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn send_live_control(&mut self, method: &str, params: Value) -> Result<Value, StructuredErrorPayload> {
        self.sync_remote_control_policy()?;
        self.send_live_control_raw(method, params)
    }

    /// Borrow the set of currently enabled per-command toolset groups.
    #[must_use]
    pub fn enabled_toolsets(&self) -> &BTreeSet<CommandGroup> {
        &self.enabled_toolsets
    }

    /// Return `true` when `group` has been enabled on this connection.
    #[must_use]
    pub fn is_toolset_enabled(&self, group: CommandGroup) -> bool {
        self.enabled_toolsets.contains(&group)
    }

    /// Enable or disable `group`, returning `true` when the active set changed.
    pub fn set_toolset_enabled(&mut self, group: CommandGroup, enabled: bool) -> bool {
        if enabled { self.enabled_toolsets.insert(group) } else { self.enabled_toolsets.remove(&group) }
    }

    /// Queue an outgoing JSON-RPC `notifications/tools/list_changed` notification for emission.
    pub fn queue_tools_list_changed(&mut self) {
        self.pending_notifications.push(json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed"
        }));
    }

    /// Drain all queued server-to-client JSON-RPC notifications in FIFO order.
    pub fn take_pending_notifications(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.pending_notifications)
    }

    /// Inspect queued outbound notifications without draining them.
    #[must_use]
    pub fn pending_notifications(&self) -> &[Value] {
        &self.pending_notifications
    }

    /// Record whether MCP `initialize` / `notifications/initialized` has been observed.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Mark this server session as initialized.
    pub fn mark_initialized(&mut self) {
        self.initialized = true;
    }

    /// Fluent builder to enforce that `initialize` must precede operational requests on this session.
    #[must_use]
    pub const fn with_require_initialize(mut self, require: bool) -> Self {
        self.require_initialize = require;
        self
    }

    /// Configure whether operational requests require an explicit prior `initialize` handshake.
    pub const fn set_require_initialize(&mut self, require: bool) {
        self.require_initialize = require;
    }

    /// Return `true` when operational requests require a prior `initialize` handshake.
    #[must_use]
    pub const fn requires_initialize(&self) -> bool {
        self.require_initialize
    }

    /// Return `true` after a `shutdown` request has been processed on this connection.
    #[must_use]
    pub const fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }

    /// Mark this server state as shutting down so subsequent operational requests are rejected.
    pub fn mark_shutting_down(&mut self) {
        self.shutting_down = true;
    }

    /// Return a cloneable handle to this server's thread-safe [`CancellationRegistry`].
    #[must_use]
    pub fn cancellation_registry(&self) -> CancellationRegistry {
        self.cancellation.clone()
    }

    /// Signal cooperative cancellation for a JSON-RPC `request_id` (as triggered by `notifications/cancelled`).
    pub fn cancel_request(&mut self, request_id: &Value) -> bool {
        self.cancellation.cancel(request_id)
    }

    pub(crate) fn current_cancel_token(&self) -> Arc<AtomicBool> {
        self.active_cancel_token.clone().unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
    }

    pub(crate) fn set_active_cancel_token(&mut self, token: Option<Arc<AtomicBool>>) -> Option<Arc<AtomicBool>> {
        std::mem::replace(&mut self.active_cancel_token, token)
    }

    /// Attach or clear an in-process [`GuiSessionHost`].
    ///
    /// If `--gui-only` is already enabled on this state, it is propagated to the newly attached
    /// host; conversely, if the attached host already enforces `--gui-only`, this state adopts it.
    pub fn set_gui_host(&mut self, host: Option<Box<dyn GuiSessionHost>>) {
        self.gui_host = host.map(|mut h| {
            if self.gui_only {
                h.set_gui_only(true);
            } else if h.is_gui_only() {
                self.gui_only = true;
                self.session.set_gui_only(GuiOnlyPolicy::Strict);
            }
            h
        });
        self.sync_policies_to_hosted_session();
    }

    /// Return `true` when an in-process [`GuiSessionHost`] is attached.
    #[must_use]
    pub fn has_gui_host(&self) -> bool {
        self.gui_host.is_some()
    }

    /// Borrow the attached [`GuiSessionHost`], if present.
    #[must_use]
    pub fn gui_host(&self) -> Option<&dyn GuiSessionHost> {
        self.gui_host.as_deref()
    }

    /// Mutably borrow the attached [`GuiSessionHost`], if present.
    pub fn gui_host_mut(&mut self) -> Option<&mut (dyn GuiSessionHost + 'static)> {
        self.gui_host.as_deref_mut()
    }

    fn apply_local_gui_only(&mut self, gui_only: bool) {
        self.gui_only = gui_only;
        let policy = if gui_only { GuiOnlyPolicy::Strict } else { GuiOnlyPolicy::Disabled };
        self.session.set_gui_only(policy);
        if let Some(host) = self.gui_host.as_mut() {
            host.set_gui_only(gui_only);
            if let Some(hosted) = host.hosted_session_mut() {
                hosted.set_gui_only(policy);
            }
        }
    }

    /// Enable or disable strict `--gui-only` enforcement, propagating to both `self.session` and
    /// any attached [`GuiSessionHost`].
    pub fn set_gui_only(&mut self, gui_only: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.control_port.is_some() {
            self.control_requested_gui_only = gui_only;
            self.control_policy_synced = false;
        }
        self.apply_local_gui_only(gui_only);
    }

    /// Return `true` when `--gui-only` policy is active on this state, session, or GUI host.
    #[must_use]
    pub fn is_gui_only(&self) -> bool {
        self.gui_only || self.session.gui_only() == GuiOnlyPolicy::Strict || self.gui_host.as_ref().is_some_and(|host| host.is_gui_only())
    }

    /// Return `true` when the progressive `gui` toolset has been explicitly enabled in normal mode.
    #[must_use]
    pub const fn is_gui_toolset_enabled(&self) -> bool {
        self.gui_toolset_enabled
    }

    /// Enable or disable the progressive `gui` toolset in normal mode, returning `true` if changed.
    pub const fn set_gui_toolset_enabled(&mut self, enabled: bool) -> bool {
        let changed = self.gui_toolset_enabled != enabled;
        self.gui_toolset_enabled = enabled;
        changed
    }
}

/// JSON Schema and behavioral annotation descriptor exposed in MCP `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    pub annotations: ToolAnnotations,
}

/// Content item returned inside an MCP `tools/call` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

/// Result envelope returned by MCP `tools/call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError")]
    pub is_error: bool,
    #[serde(rename = "structuredContent", default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
}

impl ToolCallResult {
    /// Construct a non-error text result.
    #[must_use]
    pub fn ok_text(text: impl Into<String>) -> Self {
        Self { content: vec![ContentBlock::Text { text: text.into() }], is_error: false, structured_content: None }
    }

    /// Construct a non-error pretty-printed JSON text result with matching `structuredContent`.
    pub fn ok_json(value: &impl Serialize) -> Result<Self, ToolCallError> {
        let structured = serde_json::to_value(value).map_err(|err| ToolCallError::Execution(format!("JSON serialization error: {err}")))?;
        let text = serde_json::to_string_pretty(&structured).map_err(|err| ToolCallError::Execution(format!("JSON serialization error: {err}")))?;
        Ok(Self { content: vec![ContentBlock::Text { text }], is_error: false, structured_content: Some(structured) })
    }

    /// Construct a structured domain/execution error result (`isError: true`).
    #[must_use]
    pub fn error_text(message: impl Into<String>) -> Self {
        errors::result_from_error_message(message.into())
    }

    /// Construct a canonical structured error result from an [`EngineError`].
    #[must_use]
    pub fn from_engine_error(err: &EngineError) -> Self {
        errors::result_from_engine_error(err)
    }

    /// Construct a canonical structured error result from a [`StructuredErrorPayload`].
    #[must_use]
    pub fn structured_error(payload: &StructuredErrorPayload) -> Self {
        errors::result_from_payload(payload)
    }
}

/// Protocol-level or execution-level failure returned by [`call_tool`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallError {
    /// Tool name is not currently enabled or registered.
    UnknownTool(String),
    /// Caller supplied malformed or out-of-range tool arguments (`-32602`).
    InvalidParams(String),
    /// Domain, file I/O, or engine execution error (surfaced as MCP `isError: true`).
    Execution(String),
}

impl fmt::Display for ToolCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool(name) => write!(f, "unknown tool: {name}"),
            Self::InvalidParams(msg) | Self::Execution(msg) => f.write_str(msg),
        }
    }
}

/// Return the default core MCP tool definitions (including progressive toolset discovery tools).
#[must_use]
pub fn list_tools() -> Vec<ToolDefinition> {
    let mut group_enum = vec![Value::String(String::from("all"))];
    if let Value::Array(items) = group_enum_json() {
        group_enum.extend(items);
    }
    group_enum.push(Value::String(String::from("gui")));

    let mut groups_item_enum = Vec::new();
    if let Value::Array(items) = group_enum_json() {
        groups_item_enum.extend(items);
    }
    groups_item_enum.push(Value::String(String::from("gui")));

    vec![
        ToolDefinition {
            name: String::from("doc_inspect"),
            description: String::from(concat!(
                "Inspect the active project's sheets, variables, hooks, analyses, graphs, and layouts. Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-doc_inspect.",
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sheet_id": { "type": "string", "description": "Optional specific sheet ID to inspect in full detail." },
                    "include_cells": { "type": "boolean", "description": "Include preview cell contents when inspecting a single sheet." }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::NONE.with(Effects::IDEMPOTENT)),
        },
        ToolDefinition {
            name: String::from("open_file"),
            // Keep relative manual URL here because `xtask brands` rejects `.prism` + `https://` in one string.
            description: String::from(concat!(
                "Open or import a .curvebook, .curvebook.d, .cbtemplate, .prism, .pzfx, .xlsx, .csv, or .tsv file into the session (supports .xlsx sheet listing via `list_sheets` and multi-sheet selection via `sheets`/`sheet_indices`). Docs: manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-open_file.",
            )),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "Filesystem path to open or import." },
                    "format": { "type": "string", "enum": ["curvebook", "curvebook.d", "cbtemplate", "prism", "pzfx", "xlsx", "csv", "tsv"] },
                    "target_kind": { "type": "string", "enum": ["xy", "column", "grouped", "contingency", "survival", "parts_of_whole", "multiple_variables", "nested"] },
                    "subcolumn_mode": { "type": "string" },
                    "delimiter": { "type": "string", "description": "Optional single-character delimiter override for delimited text." },
                    "sheet_id": { "type": "string", "description": "Sheet ID used when importing delimited data." },
                    "title": { "type": "string", "description": "Optional title override for imported sheet or instantiated template." },
                    "append": { "type": "boolean", "description": "When importing CSV, TSV, XLSX, or external archive/XML files, append/merge sheets into the active project instead of replacing it." },
                    "sheets": { "type": "array", "items": { "type": "string" }, "description": "Optional `.xlsx` worksheet names to import." },
                    "sheet_indices": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "description": "Optional zero-based `.xlsx` worksheet indices to import." },
                    "list_sheets": { "type": "boolean", "description": "When true for `.xlsx`, list workbook sheets without mutating the session." },
                    "has_header": { "type": "boolean", "description": "Treat top row(s) of `.xlsx` sheets as column headers (default true)." },
                    "header_rows": { "type": "integer", "minimum": 0, "maximum": 16, "description": "Number of header rows to combine per `.xlsx` sheet (default 1 when has_header is true)." },
                    "has_row_titles": { "type": "boolean", "description": "Treat first column of each `.xlsx` sheet as row titles." }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA)),
        },
        ToolDefinition {
            name: String::from("save_file"),
            description: String::from(concat!(
                "Save the current document as a .curvebook archive, .curvebook.d unpacked directory, or .cbtemplate template (atomically overwrites any existing destination target). Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-save_file.",
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Destination file path (atomically overwrites any existing target; defaults to current project path when available)." },
                    "format": { "type": "string", "enum": ["curvebook", "curvebook.d", "cbtemplate"] }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT)),
        },
        ToolDefinition {
            name: String::from("command_list"),
            description: String::from(concat!(
                "List all headless engine commands and desktop UI commands (marked with \"ui\": true). Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-command_list.",
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Case-insensitive substring filter across id, label, menu_path, and summary." },
                    "group": { "type": "string", "enum": group_enum_json(), "description": "Optional engine CommandGroup filter." },
                    "include_ui": { "type": "boolean", "description": "Include UI-only commands from ui_catalog (default: true)." },
                    "ui_only": { "type": "boolean", "description": "Return only UI-only commands (default: false)." }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::NONE.with(Effects::IDEMPOTENT)),
        },
        ToolDefinition {
            name: String::from("command_run"),
            description: String::from(concat!(
                "Execute a single engine command (or live UI command when attached to --control) with JSON parameters. Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-command_run.",
            )),
            input_schema: json!({
                "type": "object",
                "required": ["command_id"],
                "properties": {
                    "command_id": { "type": "string", "description": "Catalog command identifier (e.g. sheet.createData, table.setCell, fit.run)." },
                    "params": { "type": "object", "description": "Command parameter payload object." }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(
                Effects::MUTATES_DOCUMENT
                    .with(Effects::CHANGES_UI_STATE)
                    .with(Effects::MAY_DISCARD_DATA)
                    .with(Effects::READS_FILESYSTEM)
                    .with(Effects::WRITES_FILESYSTEM)
                    .with(Effects::LAUNCHES_PROCESS),
            ),
        },
        ToolDefinition {
            name: String::from("command_batch"),
            description: String::from(concat!(
                "Execute a sequence of commands, optionally with atomic rollback if any step fails. Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-command_batch.",
            )),
            input_schema: json!({
                "type": "object",
                "required": ["commands"],
                "properties": {
                    "commands": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["command_id"],
                            "properties": {
                                "command_id": { "type": "string" },
                                "params": { "type": "object" }
                            }
                        }
                    },
                    "atomic": { "type": "boolean", "description": "Roll back in-memory session mutations if any command in the batch fails (default: false; filesystem writes cannot be unlinked and set filesystem_side_effects_persisted: true with rolled_back: false)." }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(
                Effects::MUTATES_DOCUMENT
                    .with(Effects::CHANGES_UI_STATE)
                    .with(Effects::MAY_DISCARD_DATA)
                    .with(Effects::READS_FILESYSTEM)
                    .with(Effects::WRITES_FILESYSTEM)
                    .with(Effects::LAUNCHES_PROCESS),
            ),
        },
        ToolDefinition {
            name: String::from("render_preview"),
            description: String::from(concat!(
                "Compile a GraphSheet or LayoutSheet and rasterize a PNG preview with metadata. Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-render_preview.",
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sheet_id": { "type": "string", "description": "GraphSheet or LayoutSheet ID (defaults to active or first graph/layout)." },
                    "width_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "height_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "dpi": { "type": "integer", "minimum": 18, "maximum": 2400 }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::NONE.with(Effects::IDEMPOTENT)),
        },
        ToolDefinition {
            name: String::from("export"),
            description: String::from(concat!(
                "Export a sheet or project to svg, pdf, emf, png, tiff, csv, tsv, or json (to disk, atomically overwriting any existing file, or inline). Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-export.",
            )),
            input_schema: json!({
                "type": "object",
                "required": ["format"],
                "properties": {
                    "format": { "type": "string", "enum": ["svg", "pdf", "emf", "png", "tiff", "csv", "tsv", "json"] },
                    "sheet_id": { "type": "string" },
                    "output_path": { "type": "string", "description": "Optional destination path (atomically overwrites any existing file)." },
                    "path": { "type": "string", "description": "Alias for output_path (atomically overwrites any existing file)." },
                    "width_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "height_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "dpi": { "type": "integer", "minimum": 18, "maximum": 2400 },
                    "tiff_compression": { "type": "string", "enum": ["none", "deflate", "lzw"] },
                    "pdf_color_space": { "type": "string", "enum": ["srgb", "cmyk", "grayscale"] },
                    "pdf_version": { "type": "string", "enum": ["1.4", "1.7"], "description": "PDF specification version for PDF export (default: \"1.7\")." },
                    "ttf_font": { "type": "string", "description": "Optional path to a TrueType (.ttf) font file for PDF embedding." },
                    "all_graphs": { "type": "boolean" },
                    "svg_precision": { "type": "integer", "minimum": 1, "maximum": 8 },
                    "pretty": { "type": "boolean" },
                    "tidy": {
                        "type": "boolean",
                        "description": "Export ResultSheet in tidy long format (defaults to true when tab_id is omitted, and false when tab_id is supplied; passing tidy: true with tab_id is rejected)."
                    },
                    "tab_id": {
                        "type": "string",
                        "description": "Specific ResultSheet tab ID to export via CSV/TSV (defaults tidy to false when supplied)."
                    },
                    "csv_delimiter": { "type": "string" }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT)),
        },
        ToolDefinition {
            name: String::from("list_toolsets"),
            description: String::from(concat!(
                "List all CommandGroup toolsets, their enabled status, and normalized per-command MCP tools. Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-list_toolsets.",
            )),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::NONE.with(Effects::IDEMPOTENT)),
        },
        ToolDefinition {
            name: String::from("enable_toolset"),
            description: String::from(concat!(
                "Enable or disable progressive CommandGroup per-command tools or the 'gui' toolset (emits notifications/tools/list_changed when modified). Docs: https://hausergroup.github.io/curvebook/manual/v",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR"),
                "/registry/mcp-tools.html#mcp-tool-enable_toolset.",
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "enum": group_enum, "description": "Single CommandGroup, 'gui', or 'all' to toggle." },
                    "groups": { "type": "array", "items": { "type": "string", "enum": groups_item_enum }, "description": "Multiple CommandGroups (or 'gui') to toggle." },
                    "enabled": { "type": "boolean", "description": "Enable (true, default) or disable (false) the requested toolset(s)." }
                },
                "additionalProperties": false
            }),
            annotations: ToolAnnotations::from_effects(Effects::CHANGES_UI_STATE.with(Effects::IDEMPOTENT)),
        },
    ]
}

/// Return the exposed MCP tools for `state`.
///
/// Under `--gui-only` (`state.is_gui_only()`), this shrinks strictly to the 8 `gui` tools.
/// In normal mode, returns the 10 core tools plus per-command tools for every enabled `CommandGroup`
/// (and the 8 `gui` tools when enabled via `enable_toolset` or backed by a `GuiSessionHost`).
#[must_use]
pub fn list_tools_for_state(state: &McpServerState) -> Vec<ToolDefinition> {
    if state.is_gui_only() {
        return gui_tools::list_gui_tools();
    }
    let mut tools = list_tools();
    for spec in registry().specs() {
        if state.is_toolset_enabled(spec.group) {
            tools.push(command_spec_to_tool_definition(spec));
        }
    }
    if state.is_gui_toolset_enabled() || state.has_gui_host() {
        tools.extend(gui_tools::list_gui_tools());
    }
    tools
}

/// Dispatch an MCP `tools/call` invocation onto `state`, isolating every handler inside `run_guarded`.
pub fn call_tool(state: &mut McpServerState, name: &str, arguments: Value) -> Result<ToolCallResult, ToolCallError> {
    run_guarded_tool(name, || dispatch_tool_inner(state, name, arguments))
}

/// Dispatch an MCP `tools/call` invocation with an explicit cooperative cancellation token.
pub fn call_tool_with_cancel(
    state: &mut McpServerState,
    name: &str,
    arguments: Value,
    cancel: Arc<AtomicBool>,
) -> Result<ToolCallResult, ToolCallError> {
    let prev = state.set_active_cancel_token(Some(cancel));
    let res = run_guarded_tool(name, || dispatch_tool_inner(state, name, arguments));
    let _ = state.set_active_cancel_token(prev);
    res
}

pub(crate) fn run_guarded_tool<F>(op: &str, f: F) -> Result<ToolCallResult, ToolCallError>
where
    F: FnOnce() -> Result<ToolCallResult, ToolCallError>,
{
    match curvebook_engine::guard::run_guarded(op, || Ok(f())) {
        Ok(inner_res) => inner_res,
        Err(engine_err) => Ok(errors::result_from_engine_error(&engine_err)),
    }
}

fn dispatch_tool_inner(state: &mut McpServerState, name: &str, arguments: Value) -> Result<ToolCallResult, ToolCallError> {
    let args = arguments.as_object().ok_or_else(|| ToolCallError::InvalidParams(String::from("tool arguments must be an object")))?;

    if let Err(payload) = state.sync_remote_control_policy() {
        return Ok(errors::result_from_payload(&payload));
    }

    if gui_tools::is_gui_tool_name(name) {
        return gui_tools::dispatch_gui_tool(state, name, args);
    }

    if state.is_gui_only() {
        return Ok(gui_only_violation_result(name));
    }

    match name {
        "doc_inspect" => handle_doc_inspect(state, args),
        "open_file" => formats::handle_open_file(state, args),
        "save_file" => formats::handle_save_file(state, args),
        "command_list" => handle_command_list(state, args),
        "command_run" => handle_command_run(state, args),
        "command_batch" => handle_command_batch(state, args),
        "render_preview" => formats::handle_render_preview(state, args),
        "export" => formats::handle_export(state, args),
        "list_toolsets" => toolsets::handle_list_toolsets(state, args),
        "enable_toolset" => toolsets::handle_enable_toolset(state, args),
        _ => {
            if let Some(spec) = toolsets::find_command_spec_by_tool_name(name)
                && state.is_toolset_enabled(spec.group)
            {
                return handle_direct_command_tool(state, spec.id, Value::Object(args.clone()));
            }
            Err(ToolCallError::UnknownTool(name.to_owned()))
        }
    }
}

pub(crate) fn gui_only_violation_result(tool_name: &str) -> ToolCallResult {
    let payload = StructuredErrorPayload::new(ErrorCode::GuiOnlyViolation, format!("tool '{tool_name}' is forbidden under --gui-only policy"))
        .with_remediation(
            "Under --gui-only mode, use only the 8 human-equivalent GUI tools: ui_inspect, ui_screenshot, ui_pointer_gesture, ui_type_text, ui_press_key, ui_invoke_menu, ui_switch_tab, ui_answer_dialog.",
        )
        .with_details(json!({
            "tool": tool_name,
            "policy": "gui_only",
            "allowed_tools": gui_tools::GUI_TOOL_NAMES,
        }));
    errors::result_from_payload(&payload)
}

pub(crate) fn dispatch_gui_backend(state: &mut McpServerState, method: &str, params: Value) -> Result<Value, StructuredErrorPayload> {
    if let Some(host) = state.gui_host.as_mut() {
        return host.call_gui(method, params).map_err(|err| StructuredErrorPayload::from_engine_error(&err));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if state.control_port().is_some() {
        return state.send_live_control(method, params);
    }
    Err(StructuredErrorPayload::new(
        ErrorCode::Conflict,
        format!("GUI tool method '{method}' requires an in-process GuiSessionHost (--gui-only) or --control-port <PORT>"),
    )
    .with_remediation("Attach an in-process GuiSessionHost (e.g. desktop `--gui-only` host) or pass `--control-port <PORT>` pointing to a running CurveBook window.")
    .with_details(json!({
        "method": method,
        "gui_only": state.is_gui_only(),
    })))
}

fn handle_direct_command_tool(state: &mut McpServerState, command_id: &str, params: Value) -> Result<ToolCallResult, ToolCallError> {
    match execute_single_command(state, command_id, params) {
        Ok(val) => ToolCallResult::ok_json(&val),
        Err(err) => Ok(errors::result_from_engine_error(&err)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocInspectArgs {
    #[serde(default)]
    sheet_id: Option<String>,
    #[serde(default)]
    include_cells: Option<bool>,
}

fn handle_doc_inspect(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: DocInspectArgs = deserialize_args("doc_inspect", args)?;
    let sheet_id = trim_opt_non_empty(parsed.sheet_id, "sheet_id")?;
    let include_cells = parsed.include_cells.unwrap_or(false);

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        if sheet_id.is_none() {
            let snapshot = match state.send_live_control("state.snapshot", json!({})) {
                Ok(v) => v,
                Err(payload) => return Ok(errors::result_from_payload(&payload)),
            };
            let mut merged = Map::new();
            match state.send_live_control("doc.inspect", json!({})) {
                Ok(Value::Object(doc_summary)) => {
                    for (k, v) in doc_summary {
                        merged.insert(k, v);
                    }
                }
                Ok(_) => {}
                Err(payload) if payload.code == ErrorCode::GuiOnlyViolation => {
                    return Ok(errors::result_from_payload(&payload));
                }
                Err(_) => {}
            }
            merged.insert(String::from("mode"), Value::String(String::from("live_attach")));
            merged.insert(String::from("control_port"), json!(port));
            merged.insert(String::from("snapshot"), snapshot);
            return ToolCallResult::ok_json(&Value::Object(merged));
        }
        let params = json!({
            "sheet_id": sheet_id,
            "include_cells": include_cells,
        });
        return match state.send_live_control("doc.inspect", params) {
            Ok(mut remote) => {
                if let Some(obj) = remote.as_object_mut() {
                    if let Some(sec) = obj.get("section").and_then(Value::as_str) {
                        obj.insert(String::from("section"), Value::String(sec.to_ascii_lowercase()));
                    }
                    obj.insert(String::from("mode"), Value::String(String::from("live_attach")));
                    obj.insert(String::from("control_port"), json!(port));
                }
                ToolCallResult::ok_json(&remote)
            }
            Err(payload) => Ok(errors::result_from_payload(&payload)),
        };
    }

    let doc = state.session().document();
    if let Some(target_id) = sheet_id {
        let target = SheetId::new(target_id);
        return inspect_single_sheet(doc, &target, include_cells);
    }

    let summary = build_document_summary(state.session());
    ToolCallResult::ok_json(&summary)
}

fn inspect_single_sheet(doc: &ProjectDocument, target: &SheetId, include_cells: bool) -> Result<ToolCallResult, ToolCallError> {
    let Some(section) = doc.sheet_section(target) else {
        return Ok(errors::result_from_code(ErrorCode::NotFound, format!("sheet '{}' not found", target.as_str())));
    };
    let fetch_detail_cells = include_cells || !matches!(section, SheetSection::Data);
    let mut service_detail = match inspect_sheet_detail(doc, target, fetch_detail_cells) {
        Ok(val) => val,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    match section {
        SheetSection::Data => {
            if let Some(obj) = service_detail.as_object_mut() {
                obj.insert(String::from("section"), Value::String(String::from("data")));
                obj.remove("cols");
                if let Some(ds) = doc.data_sheet(target) {
                    obj.insert(String::from("linked_regions"), serde_json::to_value(&ds.linked_regions).unwrap_or(Value::Null));
                }
            }
            ToolCallResult::ok_json(&service_detail)
        }
        SheetSection::Info => {
            if let Some(obj) = service_detail.as_object_mut() {
                obj.insert(String::from("section"), Value::String(String::from("info")));
                if let Some(entries) = obj.remove("constants_entries") {
                    obj.insert(String::from("constants"), entries);
                }
            }
            ToolCallResult::ok_json(&service_detail)
        }
        SheetSection::Analyses => {
            let Some(rs) = doc.result_sheet(target) else {
                return Ok(errors::result_from_code(ErrorCode::NotFound, format!("sheet '{}' not found", target.as_str())));
            };
            let deps = doc.upstream_dependencies(target);
            let mut val = summarize_result_sheet_from_detail(rs, &deps, &mut service_detail);
            if include_cells
                && let Some(obj) = val.as_object_mut()
                && let Some(src) = service_detail.as_object_mut()
            {
                if let Some(tabs) = src.remove("tabs_data") {
                    obj.insert(String::from("tabs"), tabs);
                }
                if let Some(trunc) = src.remove("tabs_truncated") {
                    obj.insert(String::from("tabs_truncated"), trunc);
                }
                if let Some(checklist) = src.remove("checklist_entries") {
                    obj.insert(String::from("checklist"), checklist);
                }
                if let Some(trunc) = src.remove("checklist_truncated") {
                    obj.insert(String::from("checklist_truncated"), trunc);
                }
            }
            ToolCallResult::ok_json(&val)
        }
        SheetSection::Graphs | SheetSection::Layouts => {
            if let Some(obj) = service_detail.as_object_mut() {
                let sec = if matches!(section, SheetSection::Graphs) { "graphs" } else { "layouts" };
                obj.insert(String::from("section"), Value::String(String::from(sec)));
            }
            ToolCallResult::ok_json(&service_detail)
        }
    }
}

fn build_document_summary(session: &Session) -> Value {
    let doc = session.document();
    let Ok(project) = inspect_project_summary(doc, session.is_dirty(), session.file_path(), None) else {
        return json!({});
    };

    let data_list: Vec<Value> = project
        .data_sheets
        .iter()
        .map(|ds| {
            json!({
                "id": ds.id,
                "title": ds.title,
                "kind": ds.kind_label,
                "frozen": ds.frozen,
                "info_sheet_id": ds.info_sheet_id,
                "rows": ds.rows,
                "columns": ds.cols,
                "linked_regions_count": ds.linked_regions,
                "upstream_dependencies": ds.upstream_dependencies,
            })
        })
        .collect();

    let info_list: Vec<Value> = project
        .info_sheets
        .iter()
        .map(|info| {
            json!({
                "id": info.id,
                "title": info.title,
                "linked_data_sheets": info.linked_data_sheets,
                "constants": info.constants,
                "notes_markdown_len": info.notes_len,
            })
        })
        .collect();

    let analyses_list: Vec<Value> = doc
        .result_sheets
        .iter()
        .map(|rs| {
            let deps = doc.upstream_dependencies(&rs.id);
            let mut detail = inspect_sheet_detail(doc, &rs.id, true).unwrap_or(Value::Null);
            summarize_result_sheet_from_detail(rs, &deps, &mut detail)
        })
        .collect();

    let graphs_list: Vec<Value> = doc
        .graph_sheets
        .iter()
        .map(|gs| {
            json!({
                "id": gs.id,
                "title": gs.title,
                "kind": gs.spec.kind,
                "primary_source_sheet_id": gs.spec.primary_source_sheet_id,
                "initialized": gs.initialized,
                "canvas_size_pt": gs.spec.canvas_size_pt,
                "frame_preset": gs.spec.frame_preset,
                "series_count": gs.spec.series.len(),
                "annotations_count": gs.spec.annotations.len(),
                "comparisons_count": gs.spec.comparisons.len(),
                "upstream_dependencies": doc.upstream_dependencies(&gs.id),
            })
        })
        .collect();

    let layouts_list: Vec<Value> = doc
        .layout_sheets
        .iter()
        .map(|ls| {
            json!({
                "id": ls.id,
                "title": ls.title,
                "width": ls.spec.width,
                "height": ls.spec.height,
                "unit": ls.spec.unit,
                "page_size_pt": ls.spec.page_size_pt(),
                "orientation": ls.spec.orientation,
                "items_count": ls.spec.items.len(),
                "upstream_dependencies": doc.upstream_dependencies(&ls.id),
            })
        })
        .collect();

    let (variables, hooks) = collect_variables_and_hooks(doc);

    json!({
        "title": project.title,
        "schema_version": project.schema_version,
        "file_path": project.file_path,
        "dirty": project.dirty,
        "total_sheet_count": project.total_sheet_count,
        "active_sheet_id": project.active_sheet_id,
        "family_anchor_sheet_id": project.family_anchor_sheet_id,
        "sheets": {
            "data": data_list,
            "info": info_list,
            "analyses": analyses_list,
            "graphs": graphs_list,
            "layouts": layouts_list,
        },
        "variables": variables,
        "hooks": hooks,
        "analyses": analyses_list,
        "graphs": graphs_list,
        "layouts": layouts_list,
    })
}

fn summarize_result_sheet_from_detail(rs: &ResultSheet, deps: &[SheetId], detail: &mut Value) -> Value {
    let scalars_preview = detail.as_object_mut().and_then(|obj| obj.remove("scalars_data")).unwrap_or_else(|| json!([]));
    let scalars_truncated = detail.as_object_mut().and_then(|obj| obj.remove("scalars_truncated")).unwrap_or(Value::Bool(false));
    let mut payload = json!({
        "section": "analyses",
        "id": rs.id,
        "title": rs.title(),
        "source_sheet_id": rs.analysis.source_sheet_id,
        "selected_data_sets": rs.analysis.selected_data_sets,
        "status": rs.status,
        "has_analyzable_data": rs.analyzable_data.is_some(),
        "tabs_count": rs.tabs.len(),
        "scalars": scalars_preview,
        "scalars_truncated": scalars_truncated,
        "pairwise_comparisons_count": rs.pairwise_comparisons.len(),
        "fitted_curves_count": rs.fitted_curves.len(),
        "checklist_count": rs.checklist.len(),
        "upstream_dependencies": deps,
    });
    if matches!(&rs.analysis.kind, curvebook_doc::AnalysisKind::NonlinearRegression(nl) if nl.method == curvebook_doc::FitMethod::RobustCauchy)
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(String::from("inference_availability"), Value::String(String::from("NotApplicableInRobustMode")));
    }
    payload
}

fn collect_variables_and_hooks(doc: &ProjectDocument) -> (Value, Vec<Value>) {
    let mut info_constants = Vec::new();
    for info in &doc.info_sheets {
        for c in &info.constants {
            info_constants.push(json!({
                "info_sheet_id": info.id,
                "name": c.name,
                "value": c.value,
                "units": c.units,
            }));
        }
    }

    let mut result_scalars = Vec::new();
    let mut hooks = Vec::new();
    for rs in &doc.result_sheets {
        for s in &rs.scalars {
            result_scalars.push(json!({
                "result_sheet_id": rs.id,
                "key": s.key,
                "data_set_index": s.data_set_index,
                "value": s.value,
                "units": s.units,
            }));
        }
        for hook in rs.analysis.kind.hooked_sources() {
            hooks.push(json!({ "consumer_sheet_id": rs.id, "consumer_section": "analyses", "hook": hook }));
        }
    }

    for gs in &doc.graph_sheets {
        for hook in gs.spec.inline_hooks() {
            hooks.push(json!({ "consumer_sheet_id": gs.id, "consumer_section": "graphs", "hook": hook }));
        }
    }
    for ls in &doc.layout_sheets {
        for hook in ls.spec.inline_hooks() {
            hooks.push(json!({ "consumer_sheet_id": ls.id, "consumer_section": "layouts", "hook": hook }));
        }
    }

    let variables = json!({
        "info_constants": info_constants,
        "result_scalars": result_scalars,
    });
    (variables, hooks)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandListArgs {
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    include_ui: Option<bool>,
    #[serde(default)]
    ui_only: Option<bool>,
}

fn handle_command_list(state: &McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: CommandListArgs = deserialize_args("command_list", args)?;
    let filter = trim_opt_non_empty(parsed.filter, "filter")?.map(|s| s.to_ascii_lowercase());
    let group_filter = match trim_opt_non_empty(parsed.group, "group")? {
        Some(raw) => Some(parse_command_group(&raw)?),
        None => None,
    };
    let include_ui = parsed.include_ui.unwrap_or(true);
    let ui_only = parsed.ui_only.unwrap_or(false);
    let attached = is_attached(state);

    let mut commands = Vec::new();
    if !ui_only {
        for spec in registry().specs() {
            if group_filter.is_some_and(|g| spec.group != g) {
                continue;
            }
            if matches_filter(filter.as_deref(), spec.id, spec.label, spec.menu_path, spec.summary) {
                commands.push(json!({
                    "id": spec.id,
                    "label": spec.label,
                    "menu_path": spec.menu_path,
                    "shortcut": spec.shortcut,
                    "group": spec.group.as_str(),
                    "group_title": spec.group.title(),
                    "effects": spec.effects.flag_names(),
                    "summary": spec.summary,
                    "beginner": spec.beginner,
                    "input_schema": (spec.input_schema)(),
                    "mcp_tool": command_id_to_mcp_tool_name(spec.id),
                    "ui": false,
                    "available": true,
                }));
            }
        }
    }

    if (include_ui || ui_only) && group_filter.is_none() {
        for spec in UI_COMMANDS {
            if matches_filter(filter.as_deref(), spec.id, spec.label, "", spec.summary) {
                commands.push(json!({
                    "id": spec.id,
                    "label": spec.label,
                    "menu_path": "",
                    "shortcut": spec.shortcut,
                    "params_doc": spec.params_doc,
                    "summary": spec.summary,
                    "beginner": spec.beginner,
                    "ui": true,
                    "requires_live_attach": true,
                    "available": attached,
                }));
            }
        }
    }

    ToolCallResult::ok_json(&json!({
        "count": commands.len(),
        "live_attached": attached,
        "mode": if attached { "live_attach" } else { "headless" },
        "commands": commands,
    }))
}

fn matches_filter(filter: Option<&str>, id: &str, label: &str, menu_path: &str, summary: &str) -> bool {
    let Some(needle) = filter else {
        return true;
    };
    id.to_ascii_lowercase().contains(needle)
        || label.to_ascii_lowercase().contains(needle)
        || menu_path.to_ascii_lowercase().contains(needle)
        || summary.to_ascii_lowercase().contains(needle)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandRunArgs {
    command_id: String,
    #[serde(default)]
    params: Option<Value>,
}

fn handle_command_run(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: CommandRunArgs = deserialize_args("command_run", args)?;
    let command_id = trim_required_non_empty(parsed.command_id, "command_id")?;
    let params = validate_command_params_value(parsed.params)?;
    match execute_single_command(state, &command_id, params) {
        Ok(val) => ToolCallResult::ok_json(&val),
        Err(err) => Ok(errors::result_from_engine_error(&err)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandBatchArgs {
    commands: Vec<Value>,
    #[serde(default)]
    atomic: Option<bool>,
}

fn handle_command_batch(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let parsed: CommandBatchArgs = deserialize_args("command_batch", args)?;
    if parsed.commands.len() > MAX_BATCH_COMMANDS {
        return Err(ToolCallError::InvalidParams(format!("command_batch exceeds maximum {MAX_BATCH_COMMANDS} items")));
    }

    let atomic = parsed.atomic.unwrap_or(false);
    let mut parsed_items = Vec::with_capacity(parsed.commands.len());
    for (idx, item) in parsed.commands.into_iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(ToolCallError::InvalidParams(format!("batch item {idx} must be an object")));
        };
        let command_id = required_non_empty_str(obj, "command_id")
            .map_err(|_| ToolCallError::InvalidParams(format!("batch item {idx} requires non-empty string \"command_id\"")))?;
        let params =
            extract_command_params(obj).map_err(|_| ToolCallError::InvalidParams(format!("batch item {idx} \"params\" must be an object")))?;
        parsed_items.push((command_id, params));
    }

    execute_batch_items(state, parsed_items, atomic)
}

fn execute_batch_items(state: &mut McpServerState, items: Vec<(String, Value)>, atomic: bool) -> Result<ToolCallResult, ToolCallError> {
    #[cfg(not(target_arch = "wasm32"))]
    if state.control_port().is_some() {
        return execute_live_command_batch(state, items, atomic);
    }

    if atomic {
        return execute_headless_atomic_batch(state, items);
    }

    let mut outcomes = Vec::with_capacity(items.len());
    let cancel = state.current_cancel_token();

    for (idx, (command_id, params)) in items.into_iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            let err = EngineError::with_code(ErrorCode::Cancelled, format!("command `{command_id}` cancelled")).with_retryable(true);
            return Ok(build_non_atomic_failure_result(idx, command_id, &err, outcomes));
        }
        match execute_single_command(state, &command_id, params) {
            Ok(outcome) => {
                outcomes.push(json!({ "index": idx, "ok": true, "outcome": outcome }));
            }
            Err(err) => {
                return Ok(build_non_atomic_failure_result(idx, command_id, &err, outcomes));
            }
        }
    }

    ToolCallResult::ok_json(&json!({
        "atomic": false,
        "count": outcomes.len(),
        "results": outcomes,
    }))
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_batch_failure_header(msg: &str) -> Option<(usize, String, bool)> {
    let rest = msg.strip_prefix("batch step ")?;
    let (idx_str, rest) = rest.split_once(" (")?;
    let failed_index = idx_str.parse::<usize>().ok()?;
    let (cmd_id, rest) = rest.split_once(") failed (rolled_back=")?;
    let (rb_str, _) = rest.split_once(')')?;
    let rolled_back = match rb_str {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    Some((failed_index, cmd_id.to_owned(), rolled_back))
}

#[cfg(not(target_arch = "wasm32"))]
fn normalize_live_batch_step_outcome(cmd_id: &str, raw_outcome: Value) -> Value {
    if cmd_id == "ping" || is_ui_catalog_command(cmd_id) {
        if raw_outcome.get("command_id").is_some() && raw_outcome.get("live_attach").is_some() {
            return raw_outcome;
        }
        let mutated = raw_outcome.get("mutated").and_then(Value::as_bool).unwrap_or(cmd_id != "ping" && !is_readonly_ui_catalog_command(cmd_id));
        return json!({
            "command_id": cmd_id,
            "mutated": mutated,
            "live_attach": true,
            "result": raw_outcome,
        });
    }
    raw_outcome
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_live_command_batch(state: &mut McpServerState, mut items: Vec<(String, Value)>, atomic: bool) -> Result<ToolCallResult, ToolCallError> {
    let cancel = state.current_cancel_token();
    if cancel.load(Ordering::Acquire) {
        let first_cmd = items.first().map(|(c, _)| c.clone()).unwrap_or_default();
        let err = EngineError::with_code(ErrorCode::Cancelled, format!("command `{first_cmd}` cancelled")).with_retryable(true);
        return if atomic {
            Ok(build_atomic_failure_result(0, first_cmd, &err, true, Vec::new(), Vec::new()))
        } else {
            Ok(build_non_atomic_failure_result(0, first_cmd, &err, Vec::new()))
        };
    }

    for (idx, (command_id, params)) in items.iter_mut().enumerate() {
        if let Err(err) = formats::sanitize_command_params(state, params) {
            return if atomic {
                Ok(build_atomic_failure_result(idx, command_id.clone(), &err, true, Vec::new(), Vec::new()))
            } else {
                Ok(build_non_atomic_failure_result(idx, command_id.clone(), &err, Vec::new()))
            };
        }
    }

    let commands_json: Vec<Value> = items.iter().map(|(cmd_id, params)| json!({ "command_id": cmd_id, "params": params })).collect();

    match state.send_live_control("command.batch", json!({ "atomic": atomic, "commands": commands_json })) {
        Ok(remote) => {
            let raw_results = remote.get("results").and_then(Value::as_array);
            let mut outcomes = Vec::with_capacity(raw_results.map_or(0, Vec::len));
            if let Some(entries) = raw_results {
                for (pos, entry) in entries.iter().enumerate() {
                    let idx = entry.get("index").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(pos);
                    let cmd_id = entry
                        .get("command")
                        .or_else(|| entry.get("command_id"))
                        .and_then(Value::as_str)
                        .or_else(|| items.get(idx).map(|(c, _)| c.as_str()))
                        .unwrap_or("");
                    let raw_outcome = entry.get("result").or_else(|| entry.get("outcome")).cloned().unwrap_or(Value::Null);
                    let outcome = normalize_live_batch_step_outcome(cmd_id, raw_outcome);
                    outcomes.push(json!({ "index": idx, "ok": true, "outcome": outcome }));
                }
            }
            ToolCallResult::ok_json(&json!({
                "atomic": atomic,
                "count": outcomes.len(),
                "results": outcomes,
            }))
        }
        Err(err_payload) => {
            let parsed_header = parse_batch_failure_header(&err_payload.message);
            let detail_idx =
                err_payload.details.as_ref().and_then(|d| d.get("failed_index")).and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok());
            let detail_cmd = err_payload.details.as_ref().and_then(|d| d.get("failed_command_id")).and_then(Value::as_str).map(ToOwned::to_owned);
            let detail_rb = err_payload.details.as_ref().and_then(|d| d.get("rolled_back")).and_then(Value::as_bool);

            let failed_index = detail_idx.or_else(|| parsed_header.as_ref().map(|(idx, _, _)| *idx)).unwrap_or(0);
            let failed_command_id = detail_cmd
                .or_else(|| parsed_header.as_ref().map(|(_, cmd, _)| cmd.clone()))
                .or_else(|| items.get(failed_index).map(|(c, _)| c.clone()))
                .unwrap_or_default();
            let reported_rolled_back = detail_rb.or_else(|| parsed_header.as_ref().map(|(_, _, rb)| *rb)).or_else(|| {
                if err_payload.message.contains("(rolled_back=true)") {
                    Some(true)
                } else if err_payload.message.contains("(rolled_back=false)") {
                    Some(false)
                } else {
                    None
                }
            });

            let mut persisted_fs_steps = Vec::new();
            for (idx, (cmd_id, params)) in items.iter().take(failed_index).enumerate() {
                if command_step_writes_filesystem(cmd_id, params) {
                    persisted_fs_steps.push(json!({ "index": idx, "command_id": cmd_id }));
                }
            }

            if atomic {
                let session_rolled_back = reported_rolled_back.unwrap_or(false);
                return Ok(build_atomic_failure_result_from_payload(
                    failed_index,
                    failed_command_id,
                    &err_payload,
                    session_rolled_back,
                    Vec::new(),
                    persisted_fs_steps,
                ));
            }

            let fs_persisted = !persisted_fs_steps.is_empty();
            let mut extra = Map::new();
            extra.insert(String::from("failed_index"), json!(failed_index));
            extra.insert(String::from("failed_command_id"), json!(failed_command_id));
            extra.insert(String::from("error"), json!(err_payload.message));
            extra.insert(String::from("atomic"), json!(false));
            extra.insert(String::from("rolled_back"), json!(false));
            extra.insert(String::from("session_rolled_back"), json!(false));
            extra.insert(String::from("filesystem_side_effects_persisted"), json!(fs_persisted));
            if fs_persisted {
                extra.insert(String::from("persisted_filesystem_steps"), Value::Array(persisted_fs_steps));
            }
            extra.insert(String::from("completed"), json!(failed_index));
            extra.insert(
                String::from("results"),
                Value::Array(vec![json!({
                    "index": failed_index,
                    "ok": false,
                    "command_id": failed_command_id,
                    "error": err_payload.message,
                    "code": err_payload.code,
                })]),
            );
            Ok(errors::result_from_payload_with_extra(&err_payload, extra))
        }
    }
}

fn execute_headless_atomic_batch(state: &mut McpServerState, items: Vec<(String, Value)>) -> Result<ToolCallResult, ToolCallError> {
    let cancel = state.current_cancel_token();
    let mut steps = Vec::with_capacity(items.len());
    for (idx, (command_id, mut params)) in items.into_iter().enumerate() {
        if let Err(err) = formats::sanitize_command_params(state, &mut params) {
            return Ok(build_atomic_failure_result(idx, command_id, &err, true, Vec::new(), Vec::new()));
        }
        if is_ui_catalog_command(&command_id) {
            let err = ui_command_requires_attach_error(&command_id);
            return Ok(build_atomic_failure_result(idx, command_id, &err, true, Vec::new(), Vec::new()));
        }
        steps.push(BatchCommandStep::new(command_id, params));
    }

    let exec_result = state.mutate_active_session(|session| {
        let outcomes = session.execute_batch_atomic(&steps, Caller::Mcp, &cancel)?;
        Ok::<_, Box<curvebook_engine::services::BatchStepFailure>>((outcomes, session.is_dirty()))
    });

    match exec_result {
        Ok((outcomes, dirty)) => {
            let results: Vec<Value> = outcomes
                .iter()
                .enumerate()
                .map(|(idx, out)| json!({ "index": idx, "ok": true, "outcome": format_command_outcome_json(out, dirty) }))
                .collect();
            ToolCallResult::ok_json(&json!({
                "atomic": true,
                "count": results.len(),
                "results": results,
            }))
        }
        Err(failure) => {
            let dirty = state.session().is_dirty();
            let completed: Vec<Value> = failure
                .completed_before_failure
                .iter()
                .enumerate()
                .map(|(idx, out)| json!({ "index": idx, "ok": true, "outcome": format_command_outcome_json(out, dirty) }))
                .collect();
            let persisted_fs: Vec<Value> =
                failure.persisted_filesystem_steps.into_iter().map(|(idx, cmd_id)| json!({ "index": idx, "command_id": cmd_id })).collect();
            Ok(build_atomic_failure_result(failure.failed_index, failure.failed_command_id, &failure.error, true, completed, persisted_fs))
        }
    }
}

fn build_atomic_failure_result(
    idx: usize,
    command_id: String,
    err: &EngineError,
    session_rolled_back: bool,
    completed_before_failure: Vec<Value>,
    persisted_fs_steps: Vec<Value>,
) -> ToolCallResult {
    let err_payload = StructuredErrorPayload::from_engine_error(err);
    build_atomic_failure_result_from_payload(idx, command_id, &err_payload, session_rolled_back, completed_before_failure, persisted_fs_steps)
}

fn build_atomic_failure_result_from_payload(
    idx: usize,
    command_id: String,
    err_payload: &StructuredErrorPayload,
    session_rolled_back: bool,
    completed_before_failure: Vec<Value>,
    persisted_fs_steps: Vec<Value>,
) -> ToolCallResult {
    let mut extra = Map::new();
    extra.insert(String::from("failed_index"), json!(idx));
    extra.insert(String::from("failed_command_id"), json!(command_id));
    extra.insert(String::from("error"), json!(err_payload.message));
    let fs_persisted = !persisted_fs_steps.is_empty();
    let rolled_back = session_rolled_back && !fs_persisted;
    extra.insert(String::from("atomic"), json!(true));
    extra.insert(String::from("rolled_back"), json!(rolled_back));
    extra.insert(String::from("session_rolled_back"), json!(session_rolled_back));
    extra.insert(String::from("filesystem_side_effects_persisted"), json!(fs_persisted));
    if fs_persisted {
        extra.insert(String::from("persisted_filesystem_steps"), Value::Array(persisted_fs_steps));
        extra.insert(
            String::from("warning"),
            json!("In-memory session changes were rolled back, but earlier filesystem writes could not be reverted and remain on disk."),
        );
    }
    extra.insert(String::from("completed_before_failure"), Value::Array(completed_before_failure));
    errors::result_from_payload_with_extra(err_payload, extra)
}

fn build_non_atomic_failure_result(idx: usize, command_id: String, err: &EngineError, mut outcomes: Vec<Value>) -> ToolCallResult {
    let err_payload = StructuredErrorPayload::from_engine_error(err);
    let mut extra = Map::new();
    extra.insert(String::from("failed_index"), json!(idx));
    extra.insert(String::from("failed_command_id"), json!(command_id));
    extra.insert(String::from("error"), json!(err.message()));
    outcomes.push(json!({
        "index": idx,
        "ok": false,
        "command_id": command_id,
        "error": err.message(),
        "code": err.code,
    }));
    extra.insert(String::from("atomic"), json!(false));
    extra.insert(String::from("completed"), json!(idx));
    extra.insert(String::from("results"), Value::Array(outcomes));
    errors::result_from_payload_with_extra(&err_payload, extra)
}

fn ui_command_requires_attach_error(command_id: &str) -> EngineError {
    EngineError::with_code(ErrorCode::Conflict, format!("UI command '{command_id}' requires attaching to a live desktop --control <port>"))
        .with_remediation("Attach `--control-port <port>` to a live desktop session or invoke a headless engine command.")
}

fn format_command_outcome_json(outcome: &CommandOutcome, dirty: bool) -> Value {
    let mut payload = json!({
        "command_id": outcome.command_id,
        "mutated": outcome.mutated,
        "message": outcome.message,
        "payload": outcome.payload,
        "dirty": dirty,
    });
    if !outcome.warnings.is_empty()
        && let Some(map) = payload.as_object_mut()
    {
        map.insert(String::from("warnings"), json!(outcome.warnings));
    }
    payload
}

fn execute_single_command(state: &mut McpServerState, command_id: &str, mut params: Value) -> Result<Value, EngineError> {
    let cancel = state.current_cancel_token();
    if cancel.load(Ordering::Acquire) {
        return Err(EngineError::with_code(ErrorCode::Cancelled, format!("command `{command_id}` cancelled")).with_retryable(true));
    }

    formats::sanitize_command_params(state, &mut params)?;

    #[cfg(not(target_arch = "wasm32"))]
    if state.control_port().is_some() {
        return execute_remote_command(state, command_id, params).map_err(engine_error_from_payload);
    }

    if is_ui_catalog_command(command_id) {
        return Err(ui_command_requires_attach_error(command_id));
    }

    let (outcome, dirty) = state.mutate_active_session(|session| {
        let out = session.execute_with_caller(command_id, params, Caller::Mcp, &cancel)?;
        Ok::<_, EngineError>((out, session.is_dirty()))
    })?;
    Ok(format_command_outcome_json(&outcome, dirty))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn engine_error_from_payload(payload: StructuredErrorPayload) -> EngineError {
    let mut err = EngineError::with_code(payload.code, payload.message);
    err.retryable = payload.retryable;
    err.remediation = payload.remediation;
    err.details = payload.details;
    err
}

#[cfg(not(target_arch = "wasm32"))]
fn is_readonly_ui_catalog_command(command_id: &str) -> bool {
    matches!(
        command_id,
        "ui.screenshot" | "ui.canvas.preflight" | "ui.result.copyMethodsReport" | "ui.result.applyRemediation" | "ui.result.inspectDiagnostics"
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_remote_command(state: &mut McpServerState, command_id: &str, params: Value) -> Result<Value, StructuredErrorPayload> {
    if command_id == "ping" || is_ui_catalog_command(command_id) {
        let res = state.send_live_control(command_id, params)?;
        let mutated = res.get("mutated").and_then(Value::as_bool).unwrap_or(command_id != "ping" && !is_readonly_ui_catalog_command(command_id));
        return Ok(json!({
            "command_id": command_id,
            "mutated": mutated,
            "live_attach": true,
            "result": res,
        }));
    }
    state.send_live_control("engine.execute", json!({ "command_id": command_id, "params": params }))
}

fn is_ui_catalog_command(command_id: &str) -> bool {
    UI_COMMANDS.iter().any(|spec| spec.id == command_id)
}

const fn is_attached(state: &McpServerState) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        state.control_port.is_some()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = state;
        false
    }
}

pub(crate) fn deserialize_args<T: DeserializeOwned>(tool_name: &str, args: &Map<String, Value>) -> Result<T, ToolCallError> {
    serde_json::from_value(Value::Object(args.clone())).map_err(|err| ToolCallError::InvalidParams(format!("invalid {tool_name} arguments: {err}")))
}

pub(crate) fn trim_opt_non_empty(value: Option<String>, key: &str) -> Result<Option<String>, ToolCallError> {
    let Some(s) = value else {
        return Ok(None);
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must not be blank")));
    }
    Ok(Some(trimmed.to_owned()))
}

pub(crate) fn trim_required_non_empty(value: String, key: &str) -> Result<String, ToolCallError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!("missing or empty string parameter \"{key}\"")));
    }
    Ok(trimmed.to_owned())
}

fn validate_command_params_value(value: Option<Value>) -> Result<Value, ToolCallError> {
    match value {
        None | Some(Value::Null) => Ok(Value::Object(Map::new())),
        Some(v) if v.is_object() => Ok(v),
        Some(_) => Err(ToolCallError::InvalidParams(String::from("\"params\" must be a JSON object"))),
    }
}

pub(crate) fn extract_command_params(args: &Map<String, Value>) -> Result<Value, ToolCallError> {
    validate_command_params_value(args.get("params").cloned())
}

pub(crate) fn required_non_empty_str(args: &Map<String, Value>, key: &str) -> Result<String, ToolCallError> {
    optional_non_empty_str(args, key)?.ok_or_else(|| ToolCallError::InvalidParams(format!("missing or empty string parameter \"{key}\"")))
}

pub(crate) fn optional_non_empty_str(args: &Map<String, Value>, key: &str) -> Result<Option<String>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(s) = val.as_str() else {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a string")));
    };
    trim_opt_non_empty(Some(s.to_owned()), key)
}

pub(crate) fn optional_bool(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    val.as_bool().map(Some).ok_or_else(|| ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a boolean")))
}

pub(crate) fn optional_u32_bounded(args: &Map<String, Value>, key: &str, min: u32, max: u32) -> Result<Option<u32>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(num) = val.as_u64() else {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a positive integer")));
    };
    let parsed = u32::try_from(num).map_err(|_| ToolCallError::InvalidParams(format!("parameter \"{key}\" out of range [{min}, {max}]")))?;
    if parsed < min || parsed > max {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be in [{min}, {max}]")));
    }
    Ok(Some(parsed))
}

/// Encode raw binary data using standard RFC 4648 base64 (`=` padded).
#[must_use]
pub(crate) fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

/// Decode standard RFC 4648 base64 (`=` padded).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    if input.bytes().all(|b| b.is_ascii_whitespace()) {
        return Ok(Vec::new());
    }
    curvebook_platform::base64::decode(input).map_err(|err| err.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
mod live_control {
    use std::ffi::OsString;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::path::PathBuf;
    use std::time::Duration;

    use curvebook_doc::ErrorCode;
    use curvebook_engine::EngineError;
    use serde::Deserialize;
    use serde_json::{Map, Value};

    use crate::errors::StructuredErrorPayload;

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
    const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(30);
    const HEAVY_IO_TIMEOUT: Duration = Duration::from_secs(120);
    const MIN_CONFIGURED_IO_TIMEOUT_MS: u64 = 100;
    const MAX_CONFIGURED_IO_TIMEOUT_MS: u64 = 600_000;
    pub(crate) const MAX_CONTROL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
    const MAX_CONTROL_TOKEN_BYTES: usize = 512;
    const MAX_CONTROL_TOKEN_FILE_BYTES: usize = 1024;

    pub(crate) fn io_timeout_for_method_with_env(
        method: &str,
        env_get: &impl Fn(&str) -> Option<OsString>,
    ) -> Result<Duration, StructuredErrorPayload> {
        if let Some(raw_ms) = env_get("CURVEBOOK_CONTROL_IO_TIMEOUT_MS").filter(|v| !v.to_string_lossy().trim().is_empty()) {
            let text = raw_ms
                .to_str()
                .ok_or_else(|| StructuredErrorPayload::new(ErrorCode::BadInput, "CURVEBOOK_CONTROL_IO_TIMEOUT_MS must be valid UTF-8"))?;
            let parsed = text.trim().parse::<u64>().map_err(|_| {
                StructuredErrorPayload::new(
                    ErrorCode::BadInput,
                    format!("invalid CURVEBOOK_CONTROL_IO_TIMEOUT_MS `{text}`: expected integer milliseconds in {MIN_CONFIGURED_IO_TIMEOUT_MS}..={MAX_CONFIGURED_IO_TIMEOUT_MS}"),
                )
            })?;
            if !(MIN_CONFIGURED_IO_TIMEOUT_MS..=MAX_CONFIGURED_IO_TIMEOUT_MS).contains(&parsed) {
                return Err(StructuredErrorPayload::new(
                    ErrorCode::BadInput,
                    format!("CURVEBOOK_CONTROL_IO_TIMEOUT_MS={parsed} out of range [{MIN_CONFIGURED_IO_TIMEOUT_MS}, {MAX_CONFIGURED_IO_TIMEOUT_MS}]"),
                ));
            }
            return Ok(Duration::from_millis(parsed));
        }
        let is_heavy = matches!(
            method,
            "doc.export"
                | "command.batch"
                | "engine.execute"
                | "project.open"
                | "project.save"
                | "project.instantiateTemplate"
                | "table.importPrism"
                | "table.importPzfx"
                | "table.importXlsx"
                | "table.importCsv"
                | "render.preview"
                | "render.pdf"
                | "render.svg"
                | "ui.screenshot"
        );
        Ok(if is_heavy { HEAVY_IO_TIMEOUT } else { DEFAULT_IO_TIMEOUT })
    }

    #[derive(Debug, Deserialize)]
    struct ControlResponse {
        id: String,
        ok: bool,
        #[serde(default)]
        result: Value,
        error: Option<String>,
        #[serde(default)]
        error_detail: Option<Value>,
    }

    pub(crate) fn validate_control_token(raw: &str) -> Result<String, String> {
        if raw.is_empty() {
            return Err(String::from("control token must not be empty"));
        }
        if raw.len() > MAX_CONTROL_TOKEN_BYTES {
            return Err(format!("control token length {} exceeds maximum {MAX_CONTROL_TOKEN_BYTES} bytes", raw.len()));
        }
        if raw.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            return Err(String::from("control token must not contain whitespace or control characters"));
        }
        Ok(raw.to_owned())
    }

    #[cfg(unix)]
    fn current_unix_uid() -> Result<u32, StructuredErrorPayload> {
        use std::os::unix::fs::MetadataExt;
        static CACHED_UID: std::sync::OnceLock<Result<u32, String>> = std::sync::OnceLock::new();
        CACHED_UID
            .get_or_init(|| {
                if let Ok(meta) = std::fs::metadata("/proc/self") {
                    return Ok(meta.uid());
                }
                for bin in ["/usr/bin/id", "/bin/id", "id"] {
                    if let Ok(output) = curvebook_platform::ProcessCommand::new(bin).arg("-u").run()
                        && output.success
                    {
                        let text = String::from_utf8_lossy(&output.stdout);
                        if let Ok(uid) = text.trim().parse::<u32>() {
                            return Ok(uid);
                        }
                    }
                }
                Err(String::from("unable to resolve current OS UID for private user directory fallback"))
            })
            .clone()
            .map_err(|msg| StructuredErrorPayload::new(ErrorCode::BadInput, msg))
    }

    #[cfg(unix)]
    fn verify_unix_dir_checked(
        path: &std::path::Path,
        expected_uid: u32,
        label: &str,
        must_exist: bool,
        forbidden_mode_bits: u32,
    ) -> Result<(), (bool, StructuredErrorPayload)> {
        use std::os::unix::fs::MetadataExt;
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(err) if !must_exist && err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                let is_perm_denied = err.kind() == std::io::ErrorKind::PermissionDenied;
                return Err((
                    is_perm_denied,
                    StructuredErrorPayload::new(ErrorCode::BadInput, format!("failed to inspect {label} `{}`: {err}", path.display())),
                ));
            }
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err((
                false,
                StructuredErrorPayload::new(ErrorCode::BadInput, format!("refusing unsafe non-directory or symlink {label} `{}`", path.display())),
            ));
        }
        if meta.uid() != expected_uid {
            return Err((
                false,
                StructuredErrorPayload::new(
                    ErrorCode::BadInput,
                    format!("refusing {label} `{}` owned by uid {} (expected uid {expected_uid})", path.display(), meta.uid()),
                ),
            ));
        }
        if (meta.mode() & forbidden_mode_bits) != 0 {
            let requirement = if forbidden_mode_bits == 0o077 { "must be private 0700" } else { "must not be group/world-writable" };
            return Err((
                false,
                StructuredErrorPayload::new(
                    ErrorCode::BadInput,
                    format!("refusing {label} `{}` with permissions {:04o} ({requirement})", path.display(), meta.mode() & 0o777),
                ),
            ));
        }
        Ok(())
    }

    #[cfg(unix)]
    fn verify_unix_dir(
        path: &std::path::Path,
        expected_uid: u32,
        label: &str,
        must_exist: bool,
        forbidden_mode_bits: u32,
    ) -> Result<(), StructuredErrorPayload> {
        verify_unix_dir_checked(path, expected_uid, label, must_exist, forbidden_mode_bits).map_err(|(_, err)| err)
    }

    #[cfg(unix)]
    fn verify_private_dir(path: &std::path::Path, expected_uid: u32, label: &str, must_exist: bool) -> Result<(), StructuredErrorPayload> {
        verify_unix_dir(path, expected_uid, label, must_exist, 0o077)
    }

    #[cfg(unix)]
    fn resolve_unix_private_fallback_root(env_get: &impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, StructuredErrorPayload> {
        let uid = current_unix_uid()?;
        if let Some(raw_xdg_rt) = env_get("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            let rt_dir = PathBuf::from(raw_xdg_rt);
            verify_private_dir(&rt_dir, uid, "XDG_RUNTIME_DIR", true)?;
            let app_root = rt_dir.join("curvebook");
            verify_private_dir(&app_root, uid, "runtime directory", false)?;
            return Ok(app_root);
        }
        let tmp_base = env_get("TMPDIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let uid_root = tmp_base.join(format!("curvebook-uid-{uid}"));
        verify_private_dir(&uid_root, uid, "per-user fallback directory", false)?;
        Ok(uid_root)
    }

    fn resolve_config_dir_parts_with_env(env_get: &impl Fn(&str) -> Option<OsString>) -> Result<(PathBuf, bool), StructuredErrorPayload> {
        let nonempty_path = |key: &str| env_get(key).filter(|v| !v.is_empty()).map(PathBuf::from);
        if let Some(raw_cfg) = env_get("CURVEBOOK_CONFIG_DIR").filter(|v| !v.to_string_lossy().trim().is_empty()) {
            return Ok((PathBuf::from(raw_cfg), false));
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = nonempty_path("HOME") {
                Ok((home.join("Library").join("Application Support").join("CurveBook"), true))
            } else {
                Ok((resolve_unix_private_fallback_root(env_get)?, false))
            }
        }
        #[cfg(target_os = "windows")]
        {
            let fallback_base = nonempty_path("USERPROFILE").map(|h| h.join("AppData").join("Roaming").join("CurveBook"));
            let dir = nonempty_path("APPDATA").map(|p| p.join("CurveBook")).or(fallback_base).ok_or_else(|| {
                StructuredErrorPayload::new(
                    ErrorCode::BadInput,
                    "cannot determine CurveBook config directory: APPDATA and USERPROFILE are unset (set CURVEBOOK_CONFIG_DIR)",
                )
            })?;
            Ok((dir, false))
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if let Some(xdg) = nonempty_path("XDG_CONFIG_HOME") {
                let uid = current_unix_uid()?;
                verify_unix_dir(&xdg, uid, "XDG_CONFIG_HOME directory", false, 0o022)?;
                Ok((xdg.join("curvebook"), false))
            } else if let Some(home) = nonempty_path("HOME") {
                Ok((home.join(".config").join("curvebook"), true))
            } else {
                let uid = current_unix_uid()?;
                let root = resolve_unix_private_fallback_root(env_get)?;
                let cfg_dir = root.join("config");
                verify_private_dir(&cfg_dir, uid, "per-user config fallback directory", false)?;
                Ok((cfg_dir, false))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn resolve_config_dir_with_env(env_get: &impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, StructuredErrorPayload> {
        let (resolved, _) = resolve_config_dir_parts_with_env(env_get)?;

        #[cfg(unix)]
        {
            let uid = current_unix_uid()?;
            verify_unix_dir(&resolved, uid, "config directory", false, 0o022)?;
        }

        Ok(resolved)
    }

    #[cfg(test)]
    pub(crate) fn control_token_file_path_with_env(
        port: u16,
        env_get: &impl Fn(&str) -> Option<OsString>,
    ) -> Result<PathBuf, StructuredErrorPayload> {
        Ok(resolve_config_dir_with_env(env_get)?.join(format!("control-{port}.token")))
    }

    pub(crate) fn resolve_default_control_token(port: u16) -> Result<Option<String>, StructuredErrorPayload> {
        resolve_default_control_token_with_env(port, &|key| std::env::var_os(key))
    }

    #[cfg(unix)]
    fn read_private_control_token_file_unix(path: &std::path::Path) -> Result<Option<Vec<u8>>, StructuredErrorPayload> {
        use std::os::unix::fs::MetadataExt;
        let expected_uid = current_unix_uid()?;
        let lmeta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(StructuredErrorPayload::new(
                    ErrorCode::IoError,
                    format!("failed to inspect control token file `{}`: {err}", path.display()),
                ));
            }
        };
        if lmeta.file_type().is_symlink() || !lmeta.file_type().is_file() {
            return Err(StructuredErrorPayload::new(
                ErrorCode::BadInput,
                format!("refusing unsafe symlink or non-regular control token file `{}`", path.display()),
            ));
        }
        if lmeta.uid() != expected_uid {
            return Err(StructuredErrorPayload::new(
                ErrorCode::BadInput,
                format!("refusing control token file `{}` owned by uid {} (expected uid {expected_uid})", path.display(), lmeta.uid()),
            ));
        }
        if (lmeta.mode() & 0o077) != 0 {
            return Err(StructuredErrorPayload::new(
                ErrorCode::BadInput,
                format!(
                    "refusing control token file `{}` with group/world permissions {:04o} (must be private 0600)",
                    path.display(),
                    lmeta.mode() & 0o777
                ),
            ));
        }
        if lmeta.len() > MAX_CONTROL_TOKEN_FILE_BYTES as u64 {
            return Err(StructuredErrorPayload::new(
                ErrorCode::BadInput,
                format!("control token file `{}` exceeds maximum {MAX_CONTROL_TOKEN_FILE_BYTES} bytes", path.display()),
            ));
        }

        let file = std::fs::File::open(path).map_err(|err| {
            StructuredErrorPayload::new(ErrorCode::IoError, format!("failed to open control token file `{}`: {err}", path.display()))
        })?;
        let fmeta = file.metadata().map_err(|err| {
            StructuredErrorPayload::new(ErrorCode::IoError, format!("failed to inspect opened control token file `{}`: {err}", path.display()))
        })?;
        if !fmeta.file_type().is_file()
            || fmeta.uid() != expected_uid
            || (fmeta.mode() & 0o077) != 0
            || fmeta.dev() != lmeta.dev()
            || fmeta.ino() != lmeta.ino()
        {
            return Err(StructuredErrorPayload::new(
                ErrorCode::BadInput,
                format!("control token file `{}` failed safety checks after open", path.display()),
            ));
        }

        let mut buf = Vec::new();
        let cap = (MAX_CONTROL_TOKEN_FILE_BYTES as u64).saturating_add(1);
        file.take(cap).read_to_end(&mut buf).map_err(|err| {
            StructuredErrorPayload::new(ErrorCode::IoError, format!("failed to read control token file `{}`: {err}", path.display()))
        })?;
        if buf.len() > MAX_CONTROL_TOKEN_FILE_BYTES {
            return Err(StructuredErrorPayload::new(
                ErrorCode::BadInput,
                format!("control token file `{}` exceeds maximum {MAX_CONTROL_TOKEN_FILE_BYTES} bytes", path.display()),
            ));
        }
        Ok(Some(buf))
    }

    pub(crate) fn resolve_default_control_token_with_env(
        port: u16,
        env_get: &impl Fn(&str) -> Option<OsString>,
    ) -> Result<Option<String>, StructuredErrorPayload> {
        if let Some(raw_env) = env_get("CURVEBOOK_CONTROL_TOKEN") {
            let text =
                raw_env.to_str().ok_or_else(|| StructuredErrorPayload::new(ErrorCode::BadInput, "CURVEBOOK_CONTROL_TOKEN must be valid UTF-8"))?;
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let tok = validate_control_token(trimmed)
                    .map_err(|err| StructuredErrorPayload::new(ErrorCode::BadInput, format!("invalid CURVEBOOK_CONTROL_TOKEN: {err}")))?;
                return Ok(Some(tok));
            }
        }

        let (dir, _is_implicit_home) = resolve_config_dir_parts_with_env(env_get)?;
        let path = dir.join(format!("control-{port}.token"));
        #[cfg(unix)]
        let Some(buf) = ({
            let uid = current_unix_uid()?;
            if let Err((is_perm_denied, err)) = verify_unix_dir_checked(&dir, uid, "config directory", false, 0o022) {
                if _is_implicit_home
                    && is_perm_denied
                    && matches!(std::fs::symlink_metadata(&path), Err(meta_err) if meta_err.kind() == std::io::ErrorKind::NotFound)
                {
                    return Ok(None);
                }
                return Err(err);
            }
            read_private_control_token_file_unix(&path)?
        }) else {
            return Ok(None);
        };
        #[cfg(not(unix))]
        let buf = match curvebook_platform::fs::read_regular_file_bytes_if_exists(&path, MAX_CONTROL_TOKEN_FILE_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(None),
            Err(curvebook_platform::PlatformError::FileTooLarge { .. }) => {
                return Err(StructuredErrorPayload::new(
                    ErrorCode::BadInput,
                    format!("control token file `{}` exceeds maximum {MAX_CONTROL_TOKEN_FILE_BYTES} bytes", path.display()),
                ));
            }
            Err(curvebook_platform::PlatformError::InvalidPath { reason }) => {
                return Err(StructuredErrorPayload::new(ErrorCode::BadInput, format!("invalid control token file `{}`: {reason}", path.display())));
            }
            Err(err) => {
                return Err(StructuredErrorPayload::new(
                    ErrorCode::IoError,
                    format!("failed to read control token file `{}`: {err}", path.display()),
                ));
            }
        };
        let text = std::str::from_utf8(&buf)
            .map_err(|_| StructuredErrorPayload::new(ErrorCode::BadInput, format!("control token file `{}` is not valid UTF-8", path.display())))?;
        let trimmed = text.trim();
        let tok = validate_control_token(trimmed)
            .map_err(|err| StructuredErrorPayload::new(ErrorCode::BadInput, format!("invalid control token file `{}`: {err}", path.display())))?;
        Ok(Some(tok))
    }

    pub fn send_control_detailed_with_token(port: u16, token: Option<&str>, method: &str, params: Value) -> Result<Value, StructuredErrorPayload> {
        if port == 0 {
            return Err(StructuredErrorPayload::new(ErrorCode::BadInput, "control port must be in 1..=65535"));
        }
        let io_timeout = io_timeout_for_method_with_env(method, &|key| std::env::var_os(key))?;
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|err| {
            StructuredErrorPayload::new(ErrorCode::IoError, format!("failed to connect to desktop control port 127.0.0.1:{port}: {err}"))
                .with_retryable(true)
                .with_remediation("Ensure the desktop control server is running and reachable on 127.0.0.1.")
        })?;
        let _ = stream.set_read_timeout(Some(io_timeout));
        let _ = stream.set_write_timeout(Some(io_timeout));

        let mut writer = stream
            .try_clone()
            .map_err(|err| StructuredErrorPayload::new(ErrorCode::IoError, format!("control stream clone failed: {err}")).with_retryable(true))?;
        send_control_envelope_on_io(&mut stream, &mut writer, token, method, params, MAX_CONTROL_RESPONSE_BYTES)
    }

    #[cfg(test)]
    pub(crate) fn send_control_on_io<R: Read, W: Write>(
        reader_io: &mut R,
        writer_io: &mut W,
        method: &str,
        params: Value,
        max_response_bytes: usize,
    ) -> Result<Value, String> {
        send_control_envelope_on_io(reader_io, writer_io, None, method, params, max_response_bytes).map_err(|payload| payload.message)
    }

    #[cfg(test)]
    pub(crate) fn send_control_with_token_on_io<R: Read, W: Write>(
        reader_io: &mut R,
        writer_io: &mut W,
        token: Option<&str>,
        method: &str,
        params: Value,
        max_response_bytes: usize,
    ) -> Result<Value, StructuredErrorPayload> {
        send_control_envelope_on_io(reader_io, writer_io, token, method, params, max_response_bytes)
    }

    pub(crate) fn send_control_envelope_on_io<R: Read, W: Write>(
        reader_io: &mut R,
        writer_io: &mut W,
        token: Option<&str>,
        method: &str,
        params: Value,
        max_response_bytes: usize,
    ) -> Result<Value, StructuredErrorPayload> {
        let req_id = String::from("mcp-1");
        let mut envelope = Map::new();
        envelope.insert(String::from("id"), Value::String(req_id.clone()));
        if let Some(tok) = token {
            let validated = validate_control_token(tok).map_err(|err| StructuredErrorPayload::new(ErrorCode::BadInput, err))?;
            envelope.insert(String::from("token"), Value::String(validated));
        }
        envelope.insert(String::from("method"), Value::String(method.to_owned()));
        envelope.insert(String::from("params"), params);

        let payload = serde_json::to_vec(&Value::Object(envelope))
            .map_err(|err| StructuredErrorPayload::new(ErrorCode::InternalError, format!("failed to encode control request: {err}")))?;

        let mut reader = BufReader::new(reader_io);
        if payload.len() > 32 * 1024
            && let Some(tok) = token
        {
            let validated = validate_control_token(tok).map_err(|err| StructuredErrorPayload::new(ErrorCode::BadInput, err))?;
            let auth_env = serde_json::json!({
                "id": "mcp-auth",
                "method": "auth",
                "params": { "token": validated },
            });
            let auth_bytes = serde_json::to_vec(&auth_env)
                .map_err(|err| StructuredErrorPayload::new(ErrorCode::InternalError, format!("failed to encode control auth: {err}")))?;
            writer_io
                .write_all(&auth_bytes)
                .and_then(|()| writer_io.write_all(b"\n"))
                .and_then(|()| writer_io.flush())
                .map_err(|err| StructuredErrorPayload::new(ErrorCode::IoError, format!("control auth write failed: {err}")).with_retryable(true))?;
            let auth_line = read_bounded_line(&mut reader, max_response_bytes)
                .map_err(|msg| StructuredErrorPayload::new(ErrorCode::IoError, msg).with_retryable(true))?;
            let auth_resp: ControlResponse = serde_json::from_slice(&auth_line)
                .map_err(|err| StructuredErrorPayload::new(ErrorCode::CorruptData, format!("invalid control auth response JSON: {err}")))?;
            if auth_resp.id != "mcp-auth" {
                return Err(StructuredErrorPayload::new(ErrorCode::CorruptData, format!("mismatched control auth response id '{}'", auth_resp.id)));
            }
            if !auth_resp.ok {
                return Err(parse_control_error(auth_resp.error, auth_resp.error_detail));
            }
        }

        writer_io
            .write_all(&payload)
            .and_then(|()| writer_io.write_all(b"\n"))
            .and_then(|()| writer_io.flush())
            .map_err(|err| StructuredErrorPayload::new(ErrorCode::IoError, format!("control write failed: {err}")).with_retryable(true))?;

        let line_bytes = read_bounded_line(&mut reader, max_response_bytes)
            .map_err(|msg| StructuredErrorPayload::new(ErrorCode::IoError, msg).with_retryable(true))?;
        let resp: ControlResponse = serde_json::from_slice(&line_bytes)
            .map_err(|err| StructuredErrorPayload::new(ErrorCode::CorruptData, format!("invalid control response JSON: {err}")))?;
        if resp.id != req_id {
            return Err(StructuredErrorPayload::new(ErrorCode::CorruptData, format!("mismatched control response id '{}'", resp.id)));
        }
        if !resp.ok {
            return Err(parse_control_error(resp.error, resp.error_detail));
        }
        Ok(resp.result)
    }

    fn attach_batch_header_details(mut payload: StructuredErrorPayload, raw_error: Option<&str>) -> StructuredErrorPayload {
        if let Some(msg) = raw_error
            && let Some((failed_index, failed_command_id, rolled_back)) = super::parse_batch_failure_header(msg)
        {
            let mut details = match payload.details.take() {
                Some(Value::Object(map)) => map,
                Some(other) => {
                    let mut map = Map::new();
                    map.insert(String::from("inner"), other);
                    map
                }
                None => Map::new(),
            };
            details.entry(String::from("failed_index")).or_insert_with(|| serde_json::json!(failed_index));
            details.entry(String::from("failed_command_id")).or_insert_with(|| serde_json::json!(failed_command_id));
            details.entry(String::from("rolled_back")).or_insert_with(|| serde_json::json!(rolled_back));
            payload.details = Some(Value::Object(details));
        }
        payload
    }

    fn parse_control_error(error: Option<String>, error_detail: Option<Value>) -> StructuredErrorPayload {
        let raw_error_copy = error.clone();
        if let Some(detail) = error_detail {
            if let Ok(parsed) = serde_json::from_value::<StructuredErrorPayload>(detail.clone()) {
                return attach_batch_header_details(parsed, raw_error_copy.as_deref());
            }
            if let Ok(engine_err) = serde_json::from_value::<EngineError>(detail.clone()) {
                return attach_batch_header_details(StructuredErrorPayload::from_engine_error(&engine_err), raw_error_copy.as_deref());
            }
            if let Some(code_str) = detail.get("code").and_then(Value::as_str) {
                let code = if code_str == "unauthorized" { ErrorCode::BadInput } else { ErrorCode::from_code_str(code_str) };
                let msg = detail
                    .get("message")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(error)
                    .unwrap_or_else(|| String::from("desktop control command failed"));
                let mut payload = StructuredErrorPayload::new(code, msg);
                if let Some(retryable) = detail.get("retryable").and_then(Value::as_bool) {
                    payload = payload.with_retryable(retryable);
                }
                if let Some(rem) = detail.get("remediation").and_then(Value::as_str) {
                    payload = payload.with_remediation(rem);
                } else if code_str == "unauthorized" {
                    payload = payload.with_remediation(
                        "Pass matching --control-token <token>, set CURVEBOOK_CONTROL_TOKEN, or verify control-<port>.token in the config directory.",
                    );
                }
                if let Some(extra) = detail.get("details") {
                    payload = payload.with_details(extra.clone());
                }
                return attach_batch_header_details(payload, raw_error_copy.as_deref());
            }
        }
        let msg = error.unwrap_or_else(|| String::from("desktop control command failed"));
        let lower = msg.to_ascii_lowercase();
        if lower.contains("unauthorized") || lower.contains("invalid or missing control authentication token") {
            return StructuredErrorPayload::new(ErrorCode::BadInput, msg).with_remediation(
                "Pass matching --control-token <token>, set CURVEBOOK_CONTROL_TOKEN, or verify control-<port>.token in the config directory.",
            );
        }
        if lower.contains("gui_only_violation") || lower.contains("gui-only") || lower.contains("gui_only") {
            return StructuredErrorPayload::new(ErrorCode::GuiOnlyViolation, msg);
        }
        if lower.contains("disabled") || lower.contains("grayed") {
            return StructuredErrorPayload::new(ErrorCode::Conflict, msg);
        }
        if lower.contains("cancelled") {
            return StructuredErrorPayload::new(ErrorCode::Cancelled, msg).with_retryable(true);
        }
        StructuredErrorPayload::from_message(msg)
    }

    fn read_bounded_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        loop {
            let avail = reader.fill_buf().map_err(|err| format!("control read failed: {err}"))?;
            if avail.is_empty() {
                if buf.is_empty() {
                    return Err(String::from("desktop control connection closed without response"));
                }
                return Ok(buf);
            }
            if let Some(pos) = avail.iter().position(|&b| b == b'\n') {
                let take = pos + 1;
                if buf.len().saturating_add(take) > max_bytes {
                    return Err(format!("control response exceeds {max_bytes} bytes"));
                }
                let chunk = avail.get(..take).ok_or_else(|| String::from("short control read buffer"))?;
                buf.extend_from_slice(chunk);
                reader.consume(take);
                return Ok(buf);
            }
            if buf.len().saturating_add(avail.len()) > max_bytes {
                return Err(format!("control response exceeds {max_bytes} bytes"));
            }
            let take = avail.len();
            buf.extend_from_slice(avail);
            reader.consume(take);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use curvebook_engine::cmd::{CommandGroup, Effects, registry};
    use serde_json::{Value, json};

    use super::{McpServerState, ToolAnnotations, ToolCallError, call_tool, encode_base64, list_tools, list_tools_for_state, run_guarded_tool};
    use crate::toolsets::command_id_to_mcp_tool_name;
    use crate::ui_catalog::UI_COMMANDS;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn list_tools_exposes_all_ten_default_tools() {
        let names: Vec<String> = list_tools().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "doc_inspect",
                "open_file",
                "save_file",
                "command_list",
                "command_run",
                "command_batch",
                "render_preview",
                "export",
                "list_toolsets",
                "enable_toolset",
            ]
        );
    }

    #[test]
    fn per_command_mcp_names_are_valid_and_collision_free() {
        let mut seen: BTreeSet<String> = list_tools().into_iter().map(|t| t.name).collect();
        for gui_name in crate::gui_tools::GUI_TOOL_NAMES {
            assert!(seen.insert(gui_name.to_owned()), "collision between core tools and gui tool '{gui_name}'");
        }
        for spec in registry().specs() {
            let normalized = command_id_to_mcp_tool_name(spec.id);
            assert!(!normalized.is_empty(), "empty MCP tool name for {}", spec.id);
            assert!(
                normalized.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "invalid MCP tool name '{}' for '{}'",
                normalized,
                spec.id
            );
            assert!(seen.insert(normalized.clone()), "collision on MCP tool name '{normalized}' for '{}'", spec.id);
        }
        assert_eq!(command_id_to_mcp_tool_name("sheet.createData"), "sheet_create_data");
        assert_eq!(command_id_to_mcp_tool_name("table.setXCells"), "table_set_x_cells");
        assert_eq!(command_id_to_mcp_tool_name("graph.exportSvg"), "graph_export_svg");
    }

    #[test]
    fn annotations_match_engine_effects_flags() {
        let mut state = McpServerState::new();
        call_tool(&mut state, "enable_toolset", json!({ "group": "all" })).expect("enable all");
        let tools = list_tools_for_state(&state);
        for spec in registry().specs() {
            let mcp_name = command_id_to_mcp_tool_name(spec.id);
            let tool = tools.iter().find(|t| t.name == mcp_name).expect("per-command tool");
            let expected = ToolAnnotations {
                read_only_hint: spec.effects.read_only_hint(),
                destructive_hint: spec.effects.destructive_hint(),
                idempotent_hint: spec.effects.contains(Effects::IDEMPOTENT),
                open_world_hint: spec.effects.intersects(Effects::READS_FILESYSTEM.with(Effects::WRITES_FILESYSTEM).with(Effects::LAUNCHES_PROCESS)),
            };
            assert_eq!(tool.annotations, expected, "annotation mismatch for {}", spec.id);
        }
    }

    #[test]
    fn command_list_merges_engine_and_ui_commands_with_ui_flag() {
        let mut state = McpServerState::new();
        let res = call_tool(&mut state, "command_list", json!({})).expect("command_list");
        assert!(!res.is_error);
        assert!(res.structured_content.is_some());
        let super::ContentBlock::Text { text } = &res.content[0] else {
            panic!("expected text block");
        };
        let parsed: Value = serde_json::from_str(text).expect("json");
        let list = parsed["commands"].as_array().expect("commands array");

        for spec in UI_COMMANDS {
            let entry = list.iter().find(|c| c["id"] == spec.id).expect("ui command entry");
            assert_eq!(entry["ui"], json!(true));
            assert_eq!(entry["summary"], json!(spec.summary));
        }
        let non_ui = list.iter().find(|c| c["id"] == "sheet.createData").expect("engine command");
        assert_eq!(non_ui["ui"], json!(false));
        assert_eq!(non_ui["group"], json!(CommandGroup::Sheet.as_str()));
    }

    #[test]
    fn r1_s12_09_headless_command_list_marks_ui_unavailable() {
        let mut state = McpServerState::new();
        let res = call_tool(&mut state, "command_list", json!({})).expect("command_list");
        let parsed = res.structured_content.expect("structured_content");
        assert_eq!(parsed["live_attached"], json!(false));
        assert_eq!(parsed["mode"], json!("headless"));

        let list = parsed["commands"].as_array().expect("commands array");
        let engine_entry = list.iter().find(|c| c["id"] == "sheet.createData").expect("engine command");
        assert_eq!(engine_entry["ui"], json!(false));
        assert_eq!(engine_entry["available"], json!(true));
        assert!(engine_entry.get("requires_live_attach").is_none());

        for spec in UI_COMMANDS {
            let ui_entry = list.iter().find(|c| c["id"] == spec.id).expect("ui entry");
            assert_eq!(ui_entry["ui"], json!(true));
            assert_eq!(ui_entry["available"], json!(false));
            assert_eq!(ui_entry["requires_live_attach"], json!(true));
        }
    }

    #[test]
    fn r1_s12_07_csv_export_with_tab_id_defaults_tidy_false_and_rejects_tidy_true() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, DataSheet, DescriptiveSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
            SubcolumnMode, TableArchetype, XyTable,
        };
        use curvebook_engine::Session;

        let mut doc = ProjectDocument::new("Tab Export Doc").expect("doc");
        let xy = XyTable::new(SubcolumnMode::Single, 1, 2).expect("xy");
        let ds = DataSheet::new("ds-1", "InputData", TableArchetype::Xy(xy)).expect("ds");
        doc.data_sheets.push(ds);
        doc.active_sheet_id = Some(curvebook_doc::SheetId::new("ds-1"));

        let spec = AnalysisSpec::new("res-1", "ds-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("spec");
        let mut rs = ResultSheet::new(spec).expect("rs");
        rs.status = ResultStatus::UpToDate;
        rs.tabs.push(ResultTab {
            id: String::from("tab-alpha"),
            title: String::from("Alpha Tab"),
            kind: ResultTabKind::DescriptiveSummary,
            column_headers: vec![String::from("AlphaHeader")],
            row_labels: vec![String::from("AlphaMetric")],
            rows: vec![vec![ResultCell::Number { value: 11.0, prefix: None, units: None }]],
            footnotes: Vec::new(),
        });
        rs.tabs.push(ResultTab {
            id: String::from("tab-beta"),
            title: String::from("Beta Tab"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("BetaHeader")],
            row_labels: vec![String::from("BetaMetric")],
            rows: vec![vec![ResultCell::Number { value: 22.0, prefix: None, units: None }]],
            footnotes: Vec::new(),
        });
        doc.result_sheets.push(rs);

        let mut state = McpServerState::from_session(Session::from_document(doc));

        // Passing `sheet_id` + `tab_id` with `tidy` omitted must default `tidy` to false and export only `tab-beta`.
        let exp_beta_explicit = call_tool(&mut state, "export", json!({ "format": "csv", "sheet_id": "res-1", "tab_id": "tab-beta" }))
            .expect("export explicit sheet_id + tab-beta");
        assert!(!exp_beta_explicit.is_error, "unexpected error: {exp_beta_explicit:?}");
        let super::ContentBlock::Text { text: beta_csv } = &exp_beta_explicit.content[0] else {
            panic!("expected csv text block");
        };
        assert!(beta_csv.contains("BetaHeader") && beta_csv.contains("BetaMetric") && beta_csv.contains("22"), "got: {beta_csv}");
        assert!(!beta_csv.contains("AlphaHeader"), "unexpected alpha header in tab-beta CSV: {beta_csv}");

        // Omitting both `sheet_id` and `tidy` while active sheet is a DataSheet must resolve the ResultSheet owning `tab_id`.
        let exp_beta_implicit =
            call_tool(&mut state, "export", json!({ "format": "csv", "tab_id": "tab-beta" })).expect("export implicit sheet + tab-beta");
        assert!(!exp_beta_implicit.is_error, "unexpected error: {exp_beta_implicit:?}");
        let super::ContentBlock::Text { text: beta_implicit_csv } = &exp_beta_implicit.content[0] else {
            panic!("expected csv text block");
        };
        assert_eq!(beta_implicit_csv, beta_csv);

        let exp_alpha = call_tool(&mut state, "export", json!({ "format": "csv", "tab_id": "tab-alpha" })).expect("export tab-alpha");
        assert!(!exp_alpha.is_error, "unexpected error: {exp_alpha:?}");
        let super::ContentBlock::Text { text: alpha_csv } = &exp_alpha.content[0] else {
            panic!("expected csv text block");
        };
        assert!(alpha_csv.contains("AlphaHeader") && alpha_csv.contains("AlphaMetric") && alpha_csv.contains("11"), "got: {alpha_csv}");
        assert!(!alpha_csv.contains("BetaHeader"), "unexpected beta header in tab-alpha CSV: {alpha_csv}");

        // Omitting both `tab_id` and `tidy` on a ResultSheet preserves default tidy export across all tabs.
        let exp_tidy = call_tool(&mut state, "export", json!({ "format": "csv", "sheet_id": "res-1" })).expect("export tidy");
        assert!(!exp_tidy.is_error);
        let super::ContentBlock::Text { text: tidy_csv } = &exp_tidy.content[0] else {
            panic!("expected csv text block");
        };
        assert!(tidy_csv.contains("tab-alpha") && tidy_csv.contains("tab-beta"), "expected tidy multi-tab CSV, got: {tidy_csv}");

        // Explicit `tidy: true` + `tab_id` must be rejected with InvalidParams.
        let err = call_tool(&mut state, "export", json!({ "format": "csv", "tab_id": "tab-beta", "tidy": true }))
            .expect_err("explicit tidy: true with tab_id must fail");
        assert!(matches!(err, ToolCallError::InvalidParams(_)), "unexpected error variant: {err:?}");

        // Specifying `tab_id` while forcing `sheet_id` to a DataSheet must error clearly rather than ignoring `tab_id`.
        let wrong_sheet =
            call_tool(&mut state, "export", json!({ "format": "csv", "sheet_id": "ds-1", "tab_id": "tab-beta" })).expect("error tool call");
        assert!(wrong_sheet.is_error, "expected error for tab_id against DataSheet");
    }

    #[test]
    fn unknown_tool_and_bad_params_return_protocol_errors() {
        let mut state = McpServerState::new();
        assert!(matches!(call_tool(&mut state, "nonexistent", json!({})), Err(ToolCallError::UnknownTool(_))));
        assert!(matches!(call_tool(&mut state, "sheet_create_data", json!({})), Err(ToolCallError::UnknownTool(_))));
        assert!(matches!(call_tool(&mut state, "command_run", json!({})), Err(ToolCallError::InvalidParams(_))));
        assert!(matches!(call_tool(&mut state, "export", json!({"format": "bmp"})), Err(ToolCallError::InvalidParams(_))));
    }

    #[test]
    #[allow(clippy::panic)]
    fn run_guarded_converts_panics_into_structured_internal_panic_result() {
        let res = run_guarded_tool("test_panic", || panic!("simulated boom")).expect("recovered panic");
        assert!(res.is_error);
        let payload = res.structured_content.expect("structured error content");
        assert_eq!(payload["code"], json!("internal_panic"));
        assert_eq!(payload["category"], json!("internal"));
        assert!(payload["message"].as_str().is_some_and(|m| m.contains("simulated boom")));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn live_control_wire_protocol_validates_id_ok_and_size_caps() {
        use std::io::Cursor;

        let mut outbound = Vec::new();
        let mut inbound = Cursor::new(br#"{"id":"mcp-1","ok":true,"result":{"navigator_open":true}}"#.as_slice());
        let ok = super::live_control::send_control_on_io(&mut inbound, &mut outbound, "ui.navigator.toggle", json!({}), 1024).expect("control ok");
        assert_eq!(ok["navigator_open"], json!(true));
        let sent: Value = serde_json::from_slice(&outbound).expect("outbound jsonl");
        assert_eq!(sent, json!({"id": "mcp-1", "method": "ui.navigator.toggle", "params": {}}));

        let mut outbound = Vec::new();
        let mut bad_id = Cursor::new(br#"{"id":"wrong-id","ok":true,"result":{}}"#.as_slice());
        assert!(super::live_control::send_control_on_io(&mut bad_id, &mut outbound, "ping", json!({}), 1024).is_err());

        let mut outbound = Vec::new();
        let mut err_resp = Cursor::new(br#"{"id":"mcp-1","ok":false,"error":"denied"}"#.as_slice());
        let err = super::live_control::send_control_on_io(&mut err_resp, &mut outbound, "ping", json!({}), 1024).expect_err("control error");
        assert!(err.contains("denied"));

        let mut outbound = Vec::new();
        let huge = vec![b'a'; 256];
        let mut huge_cur = Cursor::new(huge.as_slice());
        assert!(super::live_control::send_control_on_io(&mut huge_cur, &mut outbound, "ping", json!({}), 64).is_err());

        // Detailed error parsing preserves structured error_detail codes and disabled/gui_only heuristics.
        let mut outbound = Vec::new();
        let mut gui_err = Cursor::new(
            br#"{"id":"mcp-1","ok":false,"error":"blocked","error_detail":{"code":"gui_only_violation","category":"policy","message":"forbidden shortcut","retryable":false,"remediation":"Use visible UI","details":{"remote":true}}}"#.as_slice(),
        );
        let parsed = super::live_control::send_control_envelope_on_io(&mut gui_err, &mut outbound, None, "ui.invoke_menu", json!({}), 4096)
            .expect_err("structured detail err");
        assert_eq!(parsed.code, curvebook_doc::ErrorCode::GuiOnlyViolation);
        assert_eq!(parsed.remediation.as_deref(), Some("Use visible UI"));

        let mut outbound = Vec::new();
        let mut disabled_err = Cursor::new(br#"{"id":"mcp-1","ok":false,"error":"control is disabled"}"#.as_slice());
        let fallback = super::live_control::send_control_envelope_on_io(&mut disabled_err, &mut outbound, None, "ui.switch_tab", json!({}), 4096)
            .expect_err("disabled heuristic");
        assert_eq!(fallback.code, curvebook_doc::ErrorCode::Conflict);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn live_control_token_validation_file_discovery_and_wire_envelope() {
        use std::ffi::OsString;
        use std::io::Cursor;

        assert!(McpServerState::with_control_port(41234).expect("control port").with_control_token("").is_err());
        assert!(McpServerState::with_control_port(41234).expect("control port").with_control_token("bad token with space").is_err());
        assert!(McpServerState::with_control_port(41234).expect("control port").with_control_token("bad\nline").is_err());
        let mut state = McpServerState::with_control_port(41234).expect("control port");
        state.set_control_token(Some("secret-token-42")).expect("valid token");
        assert_eq!(state.control_token(), Some("secret-token-42"));
        assert!(!format!("{state:?}").contains("secret-token-42"), "Debug output must not leak token");

        let mut outbound = Vec::new();
        let mut inbound = Cursor::new(br#"{"id":"mcp-1","ok":true,"result":{"ok":true}}"#.as_slice());
        let res = super::live_control::send_control_with_token_on_io(&mut inbound, &mut outbound, Some("secret-token-42"), "ping", json!({}), 1024)
            .expect("token ok");
        assert_eq!(res["ok"], json!(true));
        let sent: Value = serde_json::from_slice(&outbound).expect("sent json");
        assert_eq!(sent, json!({"id": "mcp-1", "token": "secret-token-42", "method": "ping", "params": {}}));

        // Unauthorized control response maps cleanly to PermissionDenied.
        let mut outbound = Vec::new();
        let mut unauth = Cursor::new(
            br#"{"id":"mcp-1","ok":false,"error":"invalid token","error_detail":{"code":"unauthorized","message":"invalid or missing control authentication token"}}"#
                .as_slice(),
        );
        let err = super::live_control::send_control_with_token_on_io(&mut unauth, &mut outbound, Some("wrong-token"), "ping", json!({}), 4096)
            .expect_err("unauthorized");
        assert_eq!(err.code, curvebook_doc::ErrorCode::BadInput);
        assert!(err.remediation.as_deref().is_some_and(|r| r.contains("--control-token")));

        // Platform token file discovery and CURVEBOOK_CONTROL_TOKEN precedence.
        let tmp_dir = std::env::temp_dir().join(format!("cb_mcp_tok_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).expect("tempdir");
        let port = 52411;
        let fake_env = |key: &str| -> Option<OsString> {
            match key {
                "HOME" | "USERPROFILE" | "APPDATA" | "XDG_CONFIG_HOME" => Some(tmp_dir.as_os_str().to_os_string()),
                _ => None,
            }
        };
        assert_eq!(super::live_control::resolve_default_control_token_with_env(port, &fake_env).expect("missing file ok"), None);

        let tok_path = super::live_control::control_token_file_path_with_env(port, &fake_env).expect("token path");
        let tok_parent = tok_path.parent().expect("parent");
        std::fs::create_dir_all(tok_parent).expect("mkdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tok_parent, std::fs::Permissions::from_mode(0o700)).expect("chmod dir 0700");
        }
        std::fs::write(&tok_path, "file-token-99\n").expect("write token file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tok_path, std::fs::Permissions::from_mode(0o600)).expect("chmod token 0600");
        }
        assert_eq!(
            super::live_control::resolve_default_control_token_with_env(port, &fake_env).expect("read token file"),
            Some(String::from("file-token-99"))
        );

        let env_override = |key: &str| -> Option<OsString> {
            if key == "CURVEBOOK_CONTROL_TOKEN" {
                return Some(OsString::from("env-token-77"));
            }
            fake_env(key)
        };
        assert_eq!(
            super::live_control::resolve_default_control_token_with_env(port, &env_override).expect("env token"),
            Some(String::from("env-token-77"))
        );

        // When HOME/XDG are unset on Unix, MCP resolves a UID-scoped path (never shared `/tmp/curvebook`)
        // and rejects symlinked fallback directories, permissive token files, and group-writable/symlinked config dirs.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // Permissive 0o644 / 0o660 / symlinked control-{port}.token files must be rejected.
            std::fs::set_permissions(&tok_path, std::fs::Permissions::from_mode(0o644)).expect("chmod 0644");
            assert!(super::live_control::resolve_default_control_token_with_env(port, &fake_env).is_err());
            std::fs::set_permissions(&tok_path, std::fs::Permissions::from_mode(0o660)).expect("chmod 0660");
            assert!(super::live_control::resolve_default_control_token_with_env(port, &fake_env).is_err());
            std::fs::set_permissions(&tok_path, std::fs::Permissions::from_mode(0o600)).expect("restore 0600");

            let real_tok_target = tmp_dir.join("real-token.txt");
            std::fs::write(&real_tok_target, "sym-token-88\n").expect("write target");
            std::fs::set_permissions(&real_tok_target, std::fs::Permissions::from_mode(0o600)).expect("chmod target 0600");
            let sym_cfg = tmp_dir.join("sym-tok-cfg");
            std::fs::create_dir_all(&sym_cfg).expect("sym cfg");
            std::fs::set_permissions(&sym_cfg, std::fs::Permissions::from_mode(0o700)).expect("chmod 0700");
            std::os::unix::fs::symlink(&real_tok_target, sym_cfg.join(format!("control-{port}.token"))).expect("symlink token");
            let sym_cfg_os = OsString::from(&sym_cfg);
            assert!(
                super::live_control::resolve_default_control_token_with_env(port, &|k| {
                    if k == "CURVEBOOK_CONFIG_DIR" { Some(sym_cfg_os.clone()) } else { None }
                })
                .is_err()
            );

            // Group-writable (0o775) and symlinked CURVEBOOK_CONFIG_DIR must be rejected.
            let gw_cfg = tmp_dir.join("gw-cfg");
            std::fs::create_dir_all(&gw_cfg).expect("gw cfg");
            std::fs::set_permissions(&gw_cfg, std::fs::Permissions::from_mode(0o775)).expect("chmod 0775");
            let gw_os = OsString::from(&gw_cfg);
            assert!(
                super::live_control::resolve_config_dir_with_env(&|k| if k == "CURVEBOOK_CONFIG_DIR" { Some(gw_os.clone()) } else { None }).is_err()
            );

            let sym_dir = tmp_dir.join("sym-cfg-dir");
            std::os::unix::fs::symlink(tok_parent, &sym_dir).expect("symlink cfg dir");
            let sym_dir_os = OsString::from(&sym_dir);
            assert!(
                super::live_control::resolve_config_dir_with_env(&|k| { if k == "CURVEBOOK_CONFIG_DIR" { Some(sym_dir_os.clone()) } else { None } })
                    .is_err()
            );

            let unset_tmp = tmp_dir.join("unset-home-tmp");
            std::fs::create_dir_all(&unset_tmp).expect("unset tmp");
            let unset_os = OsString::from(&unset_tmp);
            let uid_path = super::live_control::resolve_config_dir_with_env(&|key| {
                if key == "TMPDIR" { Some(unset_os.clone()) } else { None }
            })
            .expect("uid fallback");
            assert_ne!(uid_path, std::env::temp_dir().join("curvebook"));
            assert!(uid_path.to_string_lossy().contains("curvebook-uid-"));

            let decoy = tmp_dir.join("sym-decoy");
            std::fs::create_dir_all(&decoy).expect("decoy");
            let uid_root_name = uid_path.iter().find(|part| part.to_string_lossy().starts_with("curvebook-uid-")).expect("uid dir component");
            std::os::unix::fs::symlink(&decoy, unset_tmp.join(uid_root_name)).expect("symlink");
            assert!(super::live_control::resolve_config_dir_with_env(&|key| if key == "TMPDIR" { Some(unset_os.clone()) } else { None }).is_err());

            let rt_dir = tmp_dir.join("xdg-rt-bad");
            std::fs::create_dir_all(&rt_dir).expect("rt dir");
            std::fs::set_permissions(&rt_dir, std::fs::Permissions::from_mode(0o755)).expect("chmod 0755");
            let rt_os = OsString::from(&rt_dir);
            assert!(
                super::live_control::resolve_config_dir_with_env(&|key| if key == "XDG_RUNTIME_DIR" { Some(rt_os.clone()) } else { None }).is_err()
            );
        }

        // Verify heavy synchronous control methods receive longer IO timeouts and env override works.
        assert_eq!(super::live_control::io_timeout_for_method_with_env("ping", &|_| None).expect("ping timeout"), std::time::Duration::from_secs(30));
        assert_eq!(
            super::live_control::io_timeout_for_method_with_env("doc.export", &|_| None).expect("doc.export timeout"),
            std::time::Duration::from_secs(120)
        );
        assert_eq!(
            super::live_control::io_timeout_for_method_with_env("command.batch", &|_| None).expect("command.batch timeout"),
            std::time::Duration::from_secs(120)
        );
        assert_eq!(
            super::live_control::io_timeout_for_method_with_env("project.open", &|_| None).expect("project.open timeout"),
            std::time::Duration::from_secs(120)
        );
        assert_eq!(
            super::live_control::io_timeout_for_method_with_env("doc.export", &|k| {
                if k == "CURVEBOOK_CONTROL_IO_TIMEOUT_MS" { Some(OsString::from("45000")) } else { None }
            })
            .expect("env override timeout"),
            std::time::Duration::from_millis(45_000)
        );
        assert!(
            super::live_control::io_timeout_for_method_with_env("ping", &|k| {
                if k == "CURVEBOOK_CONTROL_IO_TIMEOUT_MS" { Some(OsString::from("0")) } else { None }
            })
            .is_err()
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn live_control_rejects_mismatched_auth_response_id() {
        use std::io::Cursor;

        // Payload > 32 KiB with token triggers pre-flight `mcp-auth` frame.
        let large_params = json!({ "blob": "x".repeat(40 * 1024) });
        let mut outbound = Vec::new();
        let mut inbound = Cursor::new(
            br#"{"id":"wrong-auth-id","ok":true,"result":{"authenticated":true}}
{"id":"mcp-1","ok":true,"result":{"ok":true}}
"#
            .as_slice(),
        );
        let err = super::live_control::send_control_with_token_on_io(
            &mut inbound,
            &mut outbound,
            Some("secret-token-42"),
            "engine.execute",
            large_params.clone(),
            4096,
        )
        .expect_err("mismatched auth_resp.id must be rejected");
        assert_eq!(err.code, curvebook_doc::ErrorCode::CorruptData);
        assert!(err.message.contains("mismatched control auth response id"), "unexpected error: {}", err.message);

        // Matching `mcp-auth` followed by matching `mcp-1` succeeds cleanly.
        let mut outbound_ok = Vec::new();
        let mut inbound_ok = Cursor::new(
            br#"{"id":"mcp-auth","ok":true,"result":{"authenticated":true}}
{"id":"mcp-1","ok":true,"result":{"accepted":true}}
"#
            .as_slice(),
        );
        let ok = super::live_control::send_control_with_token_on_io(
            &mut inbound_ok,
            &mut outbound_ok,
            Some("secret-token-42"),
            "engine.execute",
            large_params,
            4096,
        )
        .expect("matching auth_resp.id succeeds");
        assert_eq!(ok["accepted"], json!(true));
    }
}
