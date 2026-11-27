# Using CurveBook with your AI assistant

CurveBook includes both a [desktop app](reference/desktop-gui.md) and a built-in **MCP (Model Context Protocol)** server (`curvebook-cli mcp`). You can run analyses from a local AI assistant in plain English, then open and refine the same `.curvebook` project in the desktop app.

Every Beta installer includes both `curvebook` and `curvebook-cli`. Everything runs locally on your computer with zero telemetry and no cloud uploads.

> **Using CurveBook without AI?** Follow [Get started](reference/get-started.md) to install the desktop app and read the [Desktop GUI overview](reference/desktop-gui.md).

---

## What you need

1. **CurveBook installed** from the [Releases page](https://github.com/HauserGroup/curvebook/releases). Follow [Get started](reference/get-started.md) to install and verify your download.
2. **An AI desktop app that supports MCP**, such as Claude Desktop, ChatGPT Work (in the ChatGPT desktop app), Codex, Cursor, Windsurf, or another local MCP client.
3. **A folder for your data**, such as `Documents/MyResearchData`, where you keep your CSV files and `.curvebook` projects.

---

## Step 1: Locate `curvebook-cli` on your computer

Each installer places `curvebook-cli` alongside the desktop app:

- **macOS:** Drag **CurveBook.app** from the `.dmg` into **Applications**. Find the CLI binary at:
  ```text
  /Applications/CurveBook.app/Contents/MacOS/curvebook-cli
  ```
- **Windows:** Run the `.msi` installer (per-user by default). Find the CLI binary at:
  ```text
  %LocalAppData%\Programs\CurveBook\curvebook-cli.exe
  ```
  If you installed machine-wide for all users, use `%ProgramFiles%\CurveBook\curvebook-cli.exe`.
- **Linux:** Make the `.AppImage` executable and extract it once so your AI assistant can invoke `curvebook-cli` directly without unpacking on every call:
  ```sh
  chmod +x curvebook-<version>-linux-x86_64.AppImage
  ./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
  ```
  Use `aarch64` instead of `x86_64` on ARM64 Linux. The CLI binary lives at:
  ```text
  <path-to-extracted-folder>/squashfs-root/usr/bin/curvebook-cli
  ```
- **Source builds and FreeBSD:** Build with `cargo build --release -p curvebook-cli` and point your client to `target/release/curvebook-cli` (see [docs/mcp.md](mcp.md)).

---

## Step 2: Connect CurveBook to your AI app (one-time setup)

Open your AI assistant's MCP configuration file:

- **Claude Desktop (macOS):** `~/Library/Application Support/Claude/claude_desktop_config.json` (or **Settings › Developer › Edit Config**).
- **Claude Desktop (Windows):** `%APPDATA%\Claude\claude_desktop_config.json` (or **Settings › Developer › Edit Config**).
- **ChatGPT Work and Codex:** These use a TOML file instead of JSON. See [ChatGPT Work and Codex](#chatgpt-work-and-codex) below.
- **Other MCP clients (Cursor, Windsurf, Cline, and others):** Open the client's `mcp.json` or MCP server settings screen.

Add a `"curvebook"` entry matching your OS. Replace `YOURNAME` and the `--workdir` path with your real username and project folder.

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

> **Windows tip:** Use double backslashes (`\\`) inside JSON strings. Write your real username instead of `%LocalAppData%`, because most JSON configs do not expand environment variables.

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

Save the file, then **fully quit and reopen your AI app** so it loads the new `"curvebook"` server.

### ChatGPT Work and Codex {#chatgpt-work-and-codex}

ChatGPT Work (in the ChatGPT desktop app), the Codex command-line tool, and the Codex IDE extension share one settings file: `~/.codex/config.toml` (`%USERPROFILE%\.codex\config.toml` on Windows). Add CurveBook once and it appears in all three.

You can add it from the app's MCP server settings screen, or run this once in a terminal (macOS example):

```sh
codex mcp add curvebook -- /Applications/CurveBook.app/Contents/MacOS/curvebook-cli mcp --workdir /Users/YOURNAME/Documents/MyResearchData
```

Or add this to `config.toml` yourself:

```toml
[mcp_servers.curvebook]
command = "/Applications/CurveBook.app/Contents/MacOS/curvebook-cli"
args = ["mcp", "--workdir", "/Users/YOURNAME/Documents/MyResearchData"]
```

On Windows and Linux, use the same `command` and `--workdir` paths as in the JSON examples above. On Windows, single quotes avoid doubling backslashes in TOML, for example `command = 'C:\Users\YOURNAME\AppData\Local\Programs\CurveBook\curvebook-cli.exe'`.

Restart the ChatGPT desktop app or start a new Codex session to load the server. ChatGPT in a web browser cannot start programs on your computer, so it cannot use CurveBook; use the desktop app.

> **Why `--workdir`?** This flag restricts file access to that folder and its subfolders. CurveBook will not read or write files outside it.

---

## Step 3: Run your first analysis in plain English

Start a new chat in your AI app and paste this prompt:

> Use CurveBook to create a Column table comparing two groups:
> - Control: 10.2, 9.8, 10.5, 10.0, 10.1
> - Treated: 14.1, 13.8, 14.5, 13.9, 14.2
>
> Run a Welch's t-test, show me the summary statistics and P-value, create a box-and-whisker graph showing individual points, export the graph as `comparison.png` and `comparison.pdf`, and save the project as `first-example.curvebook`.

Your assistant runs CurveBook's tools locally and prints the *t*-test summary and *P*-value in chat. It also writes `comparison.png`, `comparison.pdf`, and `first-example.curvebook` into your `--workdir` folder.

---

## Step 4: Open `first-example.curvebook` in the CurveBook desktop app

Projects created through an AI assistant use the same native `.curvebook` format as the desktop app:

1. Open **CurveBook** from **Applications** on macOS, the Start Menu on Windows, or your `.AppImage` on Linux.
2. Choose **File › Open…** (`Cmd/Ctrl+O`) or drag `first-example.curvebook` from your `--workdir` folder onto the CurveBook window.
3. Use the **Project Navigator** and **Family** tree on the left to inspect your work. Click the **Data** table to view values, the **Results** sheet to check diagnostics, or the **Graph** to refine axes, error bars, themes, and page layouts.

The workflow works in reverse, too. Save edits in the desktop app, then ask your AI assistant to open the updated `.curvebook` file for follow-up fits or batch exports.

---

## Everyday things you can ask for

- **Curve fitting:** *"Fit a 4-parameter logistic (EC50) dose-response curve to this XY dataset and report IC50 with 95% confidence intervals."*
- **Model comparison:** *"Compare a 3-parameter vs 4-parameter curve fit on this table using AICc and the extra sum-of-squares F-test."*
- **Group and survival statistics:** *"Import `experiment.csv`, run a one-way ANOVA with Tukey post-hoc comparisons (or Kaplan–Meier log-rank survival analysis), and tell me which differences are significant."*
- **Publication figures:** *"Export every graph in `study.curvebook` as 300 DPI PNG and vector PDF."*
- **Existing `.pzfx files` and `.prism files`:** *"Open `old-assay.pzfx`, list its data tables, and rerun the t-test on the first one."* CurveBook can also save your data tables back out as a `.pzfx` file (data only; analyses and graphs stay in the `.curvebook` project).

---

## Troubleshooting

- **CurveBook tools do not appear:** Confirm you edited the right config file, check the `"command"` path and JSON punctuation, and **fully restart** your AI app.
- **"Access denied" or path outside `--workdir`:** CurveBook blocks paths outside your configured `--workdir` folder. Move your data files inside that folder or update `--workdir`.
- **More connection checks:** See [Troubleshooting: MCP connection](reference/troubleshooting.md#mcp-connection).
- **Technical reference:** See [MCP automation reference](reference/automation/mcp.md), [Control & batch automation](reference/automation/control-and-batch.md), and [docs/mcp.md](mcp.md).

---

## Your data stays private

- CurveBook has **no network telemetry** and never sends spreadsheets or projects to external servers.
- All computations run offline inside the `--workdir` folder you choose ([Privacy & Security](reference/privacy-security.md)).
- Your AI assistant sees only the tool calls and summary outputs shown in chat, never files outside `--workdir`.
