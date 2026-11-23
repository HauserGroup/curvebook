# Using CurveBook with Your AI Assistant

CurveBook includes both a full **[desktop app](reference/desktop-gui.md)** (`CurveBook`) and a built-in **MCP (Model Context Protocol)** server (`curvebook-cli mcp`) so you can drive analyses from a local AI assistant in plain English and inspect or refine the same `.curvebook` project in the desktop GUI.

Every Beta installer bundles both `curvebook` (desktop app) and `curvebook-cli` (CLI & MCP server) inside the same package, and everything runs locally on your computer with zero telemetry or cloud uploads.

> **Using CurveBook directly without AI?** Follow [Get Started](reference/get-started.md) to install the desktop app and browse the [Desktop App Guide](reference/desktop-gui.md).

---

## What You Need

1. **CurveBook installed** from the [Releases page](https://github.com/HauserGroup/curvebook/releases) (follow [Get Started](reference/get-started.md) to install and verify your download).
2. **An AI desktop app that supports MCP**, such as Claude Desktop, Cursor, Windsurf, or any other local MCP client.
3. **A folder for your data** — for example `Documents/MyResearchData` — where you keep your CSV/data files and `.curvebook` projects.

---

## Step 1: Locate `curvebook-cli` on Your Computer

Each CurveBook installer places `curvebook-cli` alongside the desktop application:

- **macOS:** Drag `CurveBook.app` from the `.dmg` into `/Applications`. The bundled CLI binary is at:
  ```text
  /Applications/CurveBook.app/Contents/MacOS/curvebook-cli
  ```
- **Windows:** Run the `.msi` installer (Per-User default). The bundled CLI binary is at:
  ```text
  %LocalAppData%\Programs\CurveBook\curvebook-cli.exe
  ```
  *(If you chose a machine-wide install for all users, use `%ProgramFiles%\CurveBook\curvebook-cli.exe`.)*
- **Linux:** Make the `.AppImage` executable and extract the bundled binaries once so your AI assistant can invoke `curvebook-cli` directly without unpacking the AppImage on every launch:
  ```sh
  chmod +x curvebook-<version>-linux-x86_64.AppImage
  ./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
  ```
  *(Use `aarch64` instead of `x86_64` on ARM64 Linux.)* The CLI binary is inside the extracted directory at:
  ```text
  <path-to-extracted-folder>/squashfs-root/usr/bin/curvebook-cli
  ```
- **Contributors / source checkouts / FreeBSD:** Build with `cargo build --release -p curvebook-cli` and point your AI client to `target/release/curvebook-cli` inside your checkout (see [docs/mcp.md](mcp.md)).

---

## Step 2: Connect CurveBook to Your AI App (One-Time Setup)

Open your AI assistant's MCP settings file:

- **Claude Desktop (macOS):** `~/Library/Application Support/Claude/claude_desktop_config.json` *(or **Settings → Developer → Edit Config**)*
- **Claude Desktop (Windows):** `%APPDATA%\Claude\claude_desktop_config.json` *(or **Settings → Developer → Edit Config**)*
- **Other MCP clients (Cursor, Windsurf, Cline, etc.):** Open your client's `mcp.json` or MCP server configuration screen.

Add a `"curvebook"` entry matching your OS, replacing `YOURNAME` and the `--workdir` folder path with your real username and project folder.

### macOS

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "/Applications/CurveBook.app/Contents/MacOS/curvebook-cli",
      "args": [
        "mcp",
        "--workdir",
        "/Users/YOURNAME/Documents/MyResearchData"
      ]
    }
  }
}
```

### Windows

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "C:\\Users\\YOURNAME\\AppData\\Local\\Programs\\CurveBook\\curvebook-cli.exe",
      "args": [
        "mcp",
        "--workdir",
        "C:\\Users\\YOURNAME\\Documents\\MyResearchData"
      ]
    }
  }
}
```

> **Note:** Use double backslashes (`\\`) inside Windows JSON strings, and write out your real Windows username (`YOURNAME`) instead of `%LocalAppData%` because most JSON configs do not expand environment variables.

### Linux

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "/home/YOURNAME/Applications/squashfs-root/usr/bin/curvebook-cli",
      "args": [
        "mcp",
        "--workdir",
        "/home/YOURNAME/Documents/MyResearchData"
      ]
    }
  }
}
```

Save the config file and **fully quit and reopen your AI app** so it detects the new `"curvebook"` server.

> **Why `--workdir`?** `--workdir` locks file access strictly to that folder and its subfolders. CurveBook will not read or write files outside it.

---

## Step 3: Run Your First Analysis in Plain English

Open a fresh chat in your AI app and paste a request like this:

> Use CurveBook to create a Column table comparing two groups:
> - Control: 10.2, 9.8, 10.5, 10.0, 10.1
> - Treated: 14.1, 13.8, 14.5, 13.9, 14.2
>
> Run a Welch's t-test, show me the summary statistics and P-value, create a box-and-whisker graph showing individual points, export the graph as `comparison.png` and `comparison.pdf`, and save the project as `first-example.curvebook`.

Your assistant will call CurveBook's tools locally, print the *t*-test summary and *P*-value in chat, and write `comparison.png`, `comparison.pdf`, and `first-example.curvebook` inside your `--workdir` folder.

---

## Step 4: Open `first-example.curvebook` in the CurveBook Desktop App

Projects created by an AI assistant use the exact same native `.curvebook` format as the desktop app:

1. Launch **CurveBook** (from `/Applications` on macOS, the Start Menu on Windows, or your `.AppImage` on Linux).
2. Choose **File › Open…** (`Cmd+O` on macOS / `Ctrl+O` on Windows & Linux) — or drag `first-example.curvebook` from your `--workdir` folder onto the CurveBook window — and open `first-example.curvebook`.
3. Explore the reactive **Project Navigator** and **Family** tree on the left: click the **Data** table to inspect values, click the **ResultSheet** to review diagnostics and parameter details, or click the **Graph** to customize axes, error bars, themes, and multi-panel layouts interactively.

You can also hand files back the other way: save edits in the desktop app, then ask your AI assistant to open the updated `.curvebook` file and run follow-up fits or batch exports.

---

## Everyday Things You Can Ask For

- **Curve fitting:** *"Fit a 4-parameter logistic (EC50) dose-response curve to this XY dataset and report IC50 with 95% confidence intervals."*
- **Model comparison:** *"Compare a 3-parameter vs 4-parameter curve fit on this table using AICc and the extra sum-of-squares F-test."*
- **Multi-group & survival statistics:** *"Import `experiment.csv`, run a one-way ANOVA with Tukey post-hoc comparisons (or Kaplan-Meier log-rank survival analysis), and tell me which differences are significant."*
- **Publication figures:** *"Export every graph in `study.curvebook` as 300 DPI PNG and vector PDF."*

---

## Troubleshooting

- **Your AI app does not see CurveBook tools:** Make sure you edited the right config file, checked your `--command` path and JSON commas/backslashes, and **fully restarted** the AI app.
- **"Access denied" / path outside `--workdir`:** CurveBook blocks file paths outside the `--workdir` directory you configured. Move data files inside that folder or update `--workdir`.
- **More MCP connection checks:** See [Troubleshooting: MCP Connection](reference/troubleshooting.md#mcp-connection).
- **Technical reference:** See [MCP Automation Reference](reference/automation/mcp.md), [Control & Batch Automation](reference/automation/control-and-batch.md), and [docs/mcp.md](mcp.md).

---

## Your Data Stays Private

- CurveBook has **no network telemetry** and never sends spreadsheets or projects to external servers (`cargo xtask telemetry`).
- Everything runs offline inside the `--workdir` folder you choose (see [Privacy & Security](reference/privacy-security.md)).
- Your AI assistant sees only the tool parameters and summary outputs shown in your chat — never arbitrary folders outside `--workdir`.
