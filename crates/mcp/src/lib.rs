//! Model Context Protocol (MCP) server for CurveBook (JSON-RPC 2.0 over stdio), supporting both
//! headless batch/agent sessions and live attachment to a running desktop window over the control
//! channel.
//!
//! Layer L6 crate: never depends on `curvebook-ui-egui`; UI command discovery uses the generated
//! `ui_catalog` mirror checked by `cargo xtask ui-commands --check`.
//! Governing spec: `docs/mcp.md` and `docs/implementation/scripting-and-automation.md`.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

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
pub const SERVER_NAME: &str = "curvebook";

/// Canonical base URL for the online CurveBook reference manual matching this crate minor version.
pub const DOCS_BASE_URL: &str =
    concat!("https://curvebook.org/manual/v", env!("CARGO_PKG_VERSION_MAJOR"), ".", env!("CARGO_PKG_VERSION_MINOR"), "/",);

pub use errors::StructuredErrorPayload;
pub use gui_tools::{GUI_TOOL_NAMES, is_gui_tool_name, list_gui_tools};
pub use prompts::{PromptArgument, PromptDescriptor, PromptGetResult, PromptMessage, PromptTextContent, get_prompt, list_prompts};
pub use protocol::{
    ERR_INTERNAL_ERROR, ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND, ERR_PARSE_ERROR, ERR_RESOURCE_NOT_FOUND, FramingMode,
    JsonRpcError, JsonRpcResponse, MAX_BATCH_LEN, MAX_DRAIN_BYTES, MAX_HEADER_LINE_BYTES, MAX_REQUEST_BYTES, MCP_PROTOCOL_VERSION,
    SERVER_INSTRUCTIONS, dispatch_json_value, dispatch_payload, run_stdio, run_stdio_with_state, serve, serve_with_max_bytes,
};
#[cfg(not(target_arch = "wasm32"))]
pub use protocol::{run_stdio_with_control_port, run_stdio_with_control_port_options, serve_concurrent};
pub use resources::{
    ResourceContent, ResourceDescriptor, ResourceError, ResourceTemplateDescriptor, list_resource_templates, list_resources, read_resource,
};
pub use tools::{
    CancellationRegistry, ContentBlock, GuiSessionHost, McpServerState, ToolCallError, ToolCallResult, ToolDefinition, call_tool,
    call_tool_with_cancel, list_tools, list_tools_for_state,
};
pub use toolsets::{ToolAnnotations, command_id_to_mcp_tool_name, list_tools_for_group};

#[cfg(test)]
mod tests {
    use super::{SERVER_NAME, ui_catalog::UI_COMMANDS};

    #[test]
    fn mcp_catalog_is_available() {
        assert_eq!(SERVER_NAME, "curvebook");
        assert!(!UI_COMMANDS.is_empty());
    }
}
