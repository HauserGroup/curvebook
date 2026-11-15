//! Model Context Protocol (MCP) server for LumenCraft (JSON-RPC 2.0 over stdio), supporting both
//! headless batch/agent sessions and live attachment to a running desktop window over the control
//! channel.
//!
//! Layer L6 crate: never depends on `lumencraft-ui-egui`; UI command discovery uses the generated
//! `ui_catalog` mirror checked by `cargo xtask ui-commands --check`.
//! Governing spec: `docs/mcp.md` and `docs/scripting-and-automation.md`.
#![forbid(unsafe_code)]

pub mod errors;
pub mod formats;
pub mod gui_tools;
pub mod prompts;
pub mod protocol;
pub mod resources;
pub mod tools;
pub mod toolsets;
pub mod ui_catalog;

/// MCP server protocol name advertised during initialization.
pub const SERVER_NAME: &str = "lumencraft";

pub use errors::StructuredErrorPayload;
pub use gui_tools::{GUI_TOOL_NAMES, is_gui_tool_name, list_gui_tools};
pub use prompts::{PromptArgument, PromptDescriptor, PromptGetResult, PromptMessage, PromptTextContent, get_prompt, list_prompts};
pub use protocol::{
    ERR_INTERNAL_ERROR, ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND, ERR_PARSE_ERROR, ERR_RESOURCE_NOT_FOUND, FramingMode,
    GUI_ONLY_SERVER_INSTRUCTIONS, JsonRpcError, JsonRpcResponse, MAX_BATCH_LEN, MAX_DRAIN_BYTES, MAX_REQUEST_BYTES, MCP_PROTOCOL_VERSION,
    SERVER_INSTRUCTIONS, dispatch_json_value, dispatch_payload, run_stdio, run_stdio_with_state, serve, serve_with_max_bytes,
};
#[cfg(not(target_arch = "wasm32"))]
pub use protocol::{run_stdio_with_control_port, run_stdio_with_control_port_gui_only, run_stdio_with_control_port_options};
pub use resources::{ResourceContent, ResourceDescriptor, ResourceError, list_resources, read_resource};
pub use tools::{
    ContentBlock, GuiSessionHost, McpServerState, ToolCallError, ToolCallResult, ToolDefinition, call_tool, list_tools, list_tools_for_state,
};
pub use toolsets::{ToolAnnotations, command_id_to_mcp_tool_name, list_tools_for_group};

#[cfg(test)]
mod tests {
    use super::{SERVER_NAME, ui_catalog::UI_COMMANDS};

    #[test]
    fn mcp_catalog_is_available() {
        assert_eq!(SERVER_NAME, "lumencraft");
        assert!(!UI_COMMANDS.is_empty());
    }
}
