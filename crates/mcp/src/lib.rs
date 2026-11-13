//! Model Context Protocol (MCP) server for LumenCraft (JSON-RPC 2.0 over stdio), supporting both
//! headless batch/agent sessions and live attachment to a running desktop window over the control
//! channel.
//!
//! Layer L6 crate: never depends on `lumencraft-ui-egui`; UI command discovery uses the generated
//! `ui_catalog` mirror checked by `cargo xtask ui-commands --check`.
//! Governing spec: `docs/mcp.md` and `docs/scripting-and-automation.md`.
#![forbid(unsafe_code)]

pub mod protocol;
pub mod tools;
pub mod ui_catalog;

/// MCP server protocol name advertised during initialization.
pub const SERVER_NAME: &str = "lumencraft";

#[cfg(not(target_arch = "wasm32"))]
pub use protocol::run_stdio_with_control_port;
pub use protocol::{
    ERR_INTERNAL_ERROR, ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND, ERR_PARSE_ERROR, FramingMode, JsonRpcError, JsonRpcResponse,
    MAX_BATCH_LEN, MAX_REQUEST_BYTES, MCP_PROTOCOL_VERSION, dispatch_json_value, dispatch_payload, run_stdio, serve, serve_with_max_bytes,
};
pub use tools::{ContentBlock, McpServerState, ToolCallError, ToolCallResult, ToolDefinition, call_tool, list_tools};

#[cfg(test)]
mod tests {
    use super::{SERVER_NAME, ui_catalog::UI_COMMANDS};

    #[test]
    fn mcp_catalog_is_available() {
        assert_eq!(SERVER_NAME, "lumencraft");
        assert!(!UI_COMMANDS.is_empty());
    }
}
