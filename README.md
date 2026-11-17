# CurveBook

> **New to CurveBook? [Start with the beginner's guide to using CurveBook with your AI assistant](docs/mcp-for-beginners.md).**
>
> Follow the step-by-step setup, try your first analysis, and learn how to save your work.
>
> **Technical babble ahead (temporarily):** below this section, the README gets technical. We are working on making it friendlier; the guide above is the plain-language starting point.

[![CI](https://github.com/HauserGroup/curvebook/actions/workflows/ci.yml/badge.svg)](https://github.com/HauserGroup/curvebook/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license--privacy)
[![Rust 1.99.1+](https://img.shields.io/badge/rust-1.99.1%2B-orange.svg)](#quick-start)

<p align="center">
  <img src="assets/app-icon/curvebook-win95-2508.png" alt="CurveBook logo: seven coloured curved panes meeting at a central point" width="160">
</p>

**CurveBook** is an open-source, clean-room biostatistics, nonlinear regression, and scientific graphing application written in pure Rust.

> **Status (pre-alpha):** The headless statistical, curve-fitting, and graphing engine, the command-line runner (`curvebook-cli`), and the stdio MCP server (`curvebook-cli mcp`) are implemented and working. The rendered desktop and WebAssembly GUI chrome is pre-alpha (`0/50` human-operable GUI workflows until [`R10`](docs/ROADMAP.md#r10-human-operable-gui--50-workflow-human-completion) lands); see [`docs/STATUS.md`](docs/STATUS.md) and [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Quick Start

**Prerequisites:** **Rust `1.99.1+`** (`edition = "2024"`, pinned in [`rust-toolchain.toml`](rust-toolchain.toml)). Verify both compiler and Cargo inside the repository with `rustc --version` and `cargo --version` (on macOS, if Homebrew Rust takes precedence over `rustup`, see [`docs/development.md#prerequisites`](docs/development.md#prerequisites)).

```sh
# Build the CLI (which also provides the MCP server) and inspect available commands
cargo build --release -p curvebook-cli
./target/release/curvebook-cli --help
./target/release/curvebook-cli commands --group analysis
```

For workspace tests, policy gates (`cargo xtask ci`), WebAssembly builds, and subsystem guides, see [`docs/development.md`](docs/development.md).

## Model Context Protocol (MCP)

The **Model Context Protocol (MCP)** lets an AI assistant or script talk to CurveBook's statistical and graphing engine over standard input/output (stdio) using structured JSON messages.

Key terms in this guide:
- **stdio:** Your MCP client launches `curvebook-cli mcp` as a local subprocess and exchanges JSON messages through its standard input and output streams—no background daemon or open network port is started.
- **Headless mode:** Runs calculations, file imports, and figure exports directly in memory with no GUI window (the default for `curvebook-cli mcp`).
- **`--workdir`:** Sandboxes all file reads, saves, and exports inside one chosen folder on disk so paths outside it or using `..` are rejected.

### 1. Build or Run the Server

Build the standalone release binary (recommended for MCP clients):

```sh
cargo build --release -p curvebook-cli
```

This creates `./target/release/curvebook-cli` (`.\target\release\curvebook-cli.exe` on Windows). To run directly from a repository checkout without a separate build step, use `cargo run -q -p curvebook-cli -- mcp`.

### 2. Configure Your MCP Client

Create an existing folder for your data and exports (for example `mkdir -p /path/to/your/data-folder`), then point your MCP client to the **full absolute path** of `curvebook-cli` so it does not rely on shell `PATH` or working-directory lookup (or install onto `PATH` with `cargo install --path apps/curvebook-cli` and check `which curvebook-cli`):

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "/absolute/path/to/curvebook/target/release/curvebook-cli",
      "args": ["mcp", "--workdir", "/path/to/your/data-folder"]
    }
  }
}
```

- `curvebook-cli mcp` starts in **default headless mode**.
- `--workdir` must name an existing directory; CurveBook resolves relative paths beneath it and blocks escapes (`path_escape`). Omit `--workdir` only when testing pure in-memory calculations from a terminal without disk I/O.

### 3. Verify the Connection

Send `initialize`, `notifications/initialized`, `ping`, and `tools/list` as newline-delimited JSON-RPC requests over stdin (one JSON object per line; CurveBook also auto-detects standard `Content-Length` MCP framing):

```sh
./target/release/curvebook-cli mcp <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"quickstart","version":"0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"ping"}
{"jsonrpc":"2.0","id":3,"method":"tools/list"}
EOF
```

Expected output (three JSON lines on stdout; `notifications/initialized` produces no reply):
1. Line 1 (`"id":1`): `"protocolVersion":"2024-11-05"` and `"serverInfo":{"name":"curvebook","version":"0.1.0-alpha.1"}`.
2. Line 2 (`"id":2`): exact pong `{"id":2,"jsonrpc":"2.0","result":{}}`.
3. Line 3 (`"id":3`): `"tools"` array containing the default core tools: `doc_inspect`, `open_file`, `save_file`, `command_list`, `command_run`, `command_batch`, `render_preview`, `export`, `list_toolsets`, and `enable_toolset`.

### 4. End-to-End Example (Welch's *t*-test)

Create a two-group table (`Control`: `10, 11, 12`; `Treated`: `20, 22, 24`) and run an unpaired Welch's *t*-test in one all-or-nothing (`"atomic": true`) `command_batch` call:

```sh
./target/release/curvebook-cli mcp <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"command_batch","arguments":{"atomic":true,"commands":[{"command_id":"sheet.createData","params":{"sheet_id":"assay","kind":"Column","rows":3,"cols":2}},{"command_id":"table.setTitles","params":{"sheet_id":"assay","titles":["Control","Treated"]}},{"command_id":"table.paste","params":{"sheet_id":"assay","start_row":0,"start_col":0,"text":"10\t20\n11\t22\n12\t24"}},{"command_id":"stats.run","params":{"source_sheet_id":"assay","sheet_id":"ttest","kind":"welch_t"}}]}}}
EOF
```

Expected output:
1. Line 1 (`"id":1`): `initialize` response (`"serverInfo":{"name":"curvebook","version":"0.1.0-alpha.1"}`).
2. Line 2 (`"id":2`): `tools/call` response with `"isError":false` whose final `stats.run` step (`"index":3`) reports `"message":"ran Welch's t test"`, `"status":"UpToDate"`, and these exact scalar results:
   - `"t"`: `-8.520563361656317`
   - `"PValue"`: `0.0036599555836826485`
   - `"CohensD"`: `-6.957010852370434`
   - `"HedgesG"`: `-5.550891548444351`

### 5. Common Startup & Path Troubleshooting

- **`command not found` / `ENOENT` in your MCP client:** Use the full absolute path to `./target/release/curvebook-cli` in `"command"` after running `cargo build --release -p curvebook-cli`.
- **Build fails on older Rust versions:** Run `rustc --version` and `cargo --version` inside the repository to verify both report `1.99.1` (see [`docs/development.md#prerequisites`](docs/development.md#prerequisites)).
- **`--workdir` startup error:** Ensure the `--workdir` directory exists (`mkdir -p /path/to/your/data-folder`) and is accessible before launching the server.
- **`path_escape` error on `open_file` / `save_file` / `export`:** With `--workdir` active, paths outside that folder or containing `..` are blocked; place files inside `--workdir` and pass relative paths or paths inside that directory.
- **Unexpected text breaks stdio parsing:** Keep wrapper banners and diagnostics off stdout. Cargo normally logs to stderr; `cargo run -q -p curvebook-cli -- mcp` reduces log noise. Building first and invoking `./target/release/curvebook-cli` directly simplifies startup.

For protocol framing details, attaching to a shared desktop window (`curvebook --control <port> --window` + `--control-port`), `--gui-only` mode, advanced configuration, and current limitations, see [`docs/mcp.md`](docs/mcp.md). For launch failures and empty tool lists, see [MCP connection troubleshooting](docs/mcp.md#troubleshooting-the-connection).

## Documentation & Support

- **Documentation index:** [`docs/README.md`](docs/README.md)
- **Status & Roadmap:** [`docs/STATUS.md`](docs/STATUS.md) · [`docs/ROADMAP.md`](docs/ROADMAP.md) · [`docs/target-app-parity.md`](docs/target-app-parity.md)
- **MCP, CLI & Formats:** [`docs/mcp.md`](docs/mcp.md) · [`docs/commands.md`](docs/commands.md) · [`docs/implementation/scripting-and-automation.md`](docs/implementation/scripting-and-automation.md) · [`docs/control-protocol.md`](docs/control-protocol.md) · [`docs/file-formats.md`](docs/file-formats.md)
- **Architecture & Development:** [`docs/architecture.md`](docs/architecture.md) · [`docs/development.md`](docs/development.md) · [`CONTRIBUTING.md`](CONTRIBUTING.md) · [`AGENTS.md`](AGENTS.md) · [`docs/pitfalls.md`](docs/pitfalls.md)
- **Scientific & UI Specifications:** [`docs/implementation/data-tables-and-transforms.md`](docs/implementation/data-tables-and-transforms.md) · [`docs/implementation/statistics-implementation.md`](docs/implementation/statistics-implementation.md) · [`docs/implementation/curve-fitting-implementation.md`](docs/implementation/curve-fitting-implementation.md) · [`docs/implementation/graphing-and-layouts.md`](docs/implementation/graphing-and-layouts.md) · [`docs/gui-workflows.md`](docs/gui-workflows.md)
- **Support & Security:** Open [bug](.github/ISSUE_TEMPLATE/bug_report.yml), [numerical result](.github/ISSUE_TEMPLATE/numerical_result.yml), or [feature](.github/ISSUE_TEMPLATE/feature_request.yml) reports via GitHub Issues; report security issues privately following [`SECURITY.md`](SECURITY.md).

## License & Privacy

- **License:** Dual-licensed under **MIT OR Apache-2.0** ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)). Clean-room policy, third-party notices, and contributor/asset credits live in [`ATTRIBUTION.md`](ATTRIBUTION.md), [`NOTICE`](NOTICE), [`ASSETS.md`](ASSETS.md), [`docs/licences.md`](docs/licences.md), and [`docs/contributors.md`](docs/contributors.md).
- **Zero Telemetry:** CurveBook collects zero telemetry, analytics, or crash reports and runs strictly offline (`cargo xtask telemetry`).
