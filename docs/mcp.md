# Model Context Protocol Server (`lumencraft-mcp`)

`lumencraft-cli mcp` runs a stdio [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server (`crates/mcp`) over an in-memory `lumencraft-engine` session (or attached to a live desktop window via `--control`).

## Client Configuration

```json
{
  "mcpServers": {
    "lumencraft": {
      "command": "lumencraft-cli",
      "args": ["mcp"]
    }
  }
}
```

> **Status:** Implemented and tested (`cargo test -p lumencraft-mcp --offline`: `25 passed, 0 failed`), exposing `8` MCP tools over `75` engine commands and `17` synced desktop `UI_COMMANDS`.

## Tool Surface

| Tool Category | Purpose |
|---|---|
| `doc_inspect` / `open_file` / `save_file` | Inspect project sheets (`Data`, `Info`, `Results`, `Graphs`, `Layouts`) or live `--control` state snapshots, load `.lumencraft` / `.lctemplate` / `.prism` / `.pzfx` / `.csv` / `.tsv`, and save `.lumencraft` or `.lctemplate` projects. |
| `command_list` / `command_run` / `command_batch` | Discover and execute `75` engine commands plus `17` UI commands transactionally (with optional `atomic` rollback): create data tables, populate cells, exclude outliers, run statistical tests, fit nonlinear equations, style axes, add significance brackets. |
| `render_preview` / `export` | Compile and rasterize any `GraphSheet` or `LayoutSheet` to PNG preview bytes for multimodal inspection, or export publication `SVG` / `%PDF-1.7` / `PNG` / `TIFF` / `CSV` / `JSON` files. |

## UI Command Sync

Desktop-only shell commands declared in `crates/ui-egui/src/menus/ui_commands.rs` are mirrored into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (verified in CI with `cargo xtask ui-commands --check`) so `lumencraft-mcp` never depends directly on `lumencraft-ui-egui`.
