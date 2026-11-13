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

## Planned Tool Surface

| Tool Category | Purpose |
|---|---|
| `doc_inspect` / `open_file` / `save_file` | Inspect project sheets (`Data`, `Info`, `Results`, `Graphs`, `Layouts`), load `.lumencraft` / CSV / TSV / `.pzfx`, and save projects. |
| `command_list` / `command_run` / `command_batch` | Discover and execute engine commands transactionally (create data tables, populate cells, exclude outliers, run statistical tests, fit nonlinear equations, style axes, add significance brackets). |
| `render_preview` / `export` | Render any Graph or Layout sheet to PNG preview bytes for visual inspection by multimodal agents, or export publication SVG / PDF / TIFF files. |

## UI Command Sync

Desktop-only shell commands declared in `crates/ui-egui/src/menus/ui_commands.rs` are mirrored into `crates/mcp/src/ui_catalog.rs` via `cargo xtask ui-commands` (verified in CI with `cargo xtask ui-commands --check`) so `lumencraft-mcp` never depends directly on `lumencraft-ui-egui`.
