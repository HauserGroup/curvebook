# Using CurveBook with your AI assistant

<!-- Audience: Tier 1 Scientist Guide — keep sentences short and plain; put full MCP tool schemas, flags, and protocol details in docs/reference/automation/mcp.md. -->

CurveBook includes both a [desktop app](reference/desktop-gui.md) and a built-in **MCP (Model Context Protocol)** server. You can ask a local AI assistant to run analyses in plain English, then open and polish the saved `.curvebook` project in the desktop app.

Every desktop installer bundles both the app and `curvebook-cli`. Everything runs offline on your computer with zero telemetry and no cloud uploads.

> **Using CurveBook without AI?** Follow [Get started](reference/get-started.md) to install the desktop app and read the [Desktop GUI overview](reference/desktop-gui.md).

---

## What you need

1. **CurveBook installed** from the [Releases page](https://github.com/HauserGroup/curvebook/releases). Follow [Get started](reference/get-started.md) to install and verify your download.
2. **An MCP-compatible desktop AI app**, such as Claude Desktop, ChatGPT Work (in the ChatGPT desktop app), Codex, Cursor, or Windsurf.
3. **A folder for your data**, such as `Documents/MyResearchData`, for your CSV files and `.curvebook` projects.

---

## Step 1: Locate curvebook-cli on your computer

Each installer places `curvebook-cli` alongside the desktop app:

- **macOS:** Drag **CurveBook.app** from the disk image into **Applications**. Find the CLI binary at:
  ```text
  /Applications/CurveBook.app/Contents/MacOS/curvebook-cli
  ```
- **Windows:** Run the Windows installer (per-user by default). Find the CLI binary at:
  ```text
  %LocalAppData%\Programs\CurveBook\curvebook-cli.exe
  ```
  For machine-wide installs, use `%ProgramFiles%\CurveBook\curvebook-cli.exe`.
- **Linux:** Make the AppImage executable and extract it once so your AI app can run the CLI without unpacking each time:
  ```sh
  chmod +x curvebook-<version>-linux-x86_64.AppImage
  ./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
  ```
  Use the `aarch64` file on ARM64 Linux. The CLI binary lives at:
  ```text
  <path-to-extracted-folder>/squashfs-root/usr/bin/curvebook-cli
  ```
- **FreeBSD and source builds:** See [FreeBSD install](reference/get-started.md#install-freebsd) or the [developer MCP guide](mcp.md).

---

## Step 2: Connect CurveBook to your AI app (one-time setup)

Open your AI assistant's MCP configuration file:

- **Claude Desktop (macOS):** `~/Library/Application Support/Claude/claude_desktop_config.json` (or **Settings › Developer › Edit Config**).
- **Claude Desktop (Windows):** `%APPDATA%\Claude\claude_desktop_config.json` (or **Settings › Developer › Edit Config**).
- **ChatGPT Work and Codex:** These use a TOML file instead of JSON. See [ChatGPT Work and Codex](#chatgpt-work-and-codex) below.
- **Other MCP clients (Cursor, Windsurf, Cline):** Open your client's MCP server settings or `mcp.json` file.

Add a **curvebook** entry for your operating system. Replace `YOURNAME` and the `--workdir` path with your real username and data folder.

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

> **Windows tip:** Use double backslashes (`\\`) inside JSON strings. Write your real username instead of `%LocalAppData%`, because JSON configs do not expand environment variables.

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

Save the file, then **fully quit and reopen your AI app** so it loads CurveBook.

### ChatGPT Work and Codex {#chatgpt-work-and-codex}

ChatGPT Work (in the ChatGPT desktop app), the Codex CLI, and the Codex IDE extension share one settings file: `~/.codex/config.toml` (or `%USERPROFILE%\.codex\config.toml` on Windows). Add CurveBook once to enable it in all three.

Add it in the app's MCP server settings, or run this once in a terminal (macOS example):

```sh
codex mcp add curvebook -- /Applications/CurveBook.app/Contents/MacOS/curvebook-cli mcp --workdir /Users/YOURNAME/Documents/MyResearchData
```

Or add this block to `config.toml`:

```toml
[mcp_servers.curvebook]
command = "/Applications/CurveBook.app/Contents/MacOS/curvebook-cli"
args = ["mcp", "--workdir", "/Users/YOURNAME/Documents/MyResearchData"]
```

On Windows and Linux, use the paths from the JSON examples above. In TOML on Windows, single quotes let you write single backslashes (such as `command = 'C:\Users\YOURNAME\...'`).

Restart the ChatGPT desktop app or start a new Codex session to load CurveBook. ChatGPT in a web browser cannot start local programs, so use the desktop app.

> **Why `--workdir`?** This flag limits CurveBook to that folder and its subfolders. It will not read or write files outside it.

---

## Step 3: Run your first analysis in plain English

Start a new chat in your AI app and paste this prompt:

> Use CurveBook to create a Column table comparing two groups:
> - Control: 10.2, 9.8, 10.5, 10.0, 10.1
> - Treated: 14.1, 13.8, 14.5, 13.9, 14.2
>
> Run a Welch's t-test and show the summary statistics and P-value. Create a box-and-whisker graph with individual points, export comparison.png and comparison.pdf, and save first-example.curvebook.

Your assistant runs CurveBook locally and prints the *t*-test summary and *P*-value in chat. It saves `comparison.png`, `comparison.pdf`, and `first-example.curvebook` in your workspace folder.

---

## Step 4: Open the project in the desktop app

AI assistants save standard `.curvebook` files you can open directly in the desktop app:

1. Open **CurveBook** from **Applications** (macOS), the Start Menu (Windows), or your AppImage (Linux).
2. Choose **File › Open…** (`Cmd/Ctrl+O`) or drag `first-example.curvebook` onto the CurveBook window.
3. Use the **Project Navigator** and **Family** tree on the left to browse your work. Open the **Data** table to check values, **Results** to inspect diagnostics, or **Graph** to polish axes, error bars, and layouts.

This works in reverse, too. Edit and save a project in the desktop app, then ask your AI assistant to run follow-up fits or batch exports.

---

## Everyday things you can ask for

- **Curve fitting:** *"Fit a 4-parameter logistic (EC50) curve to this XY table and report IC50 with 95% confidence intervals."*
- **Model comparison:** *"Compare a 3-parameter and 4-parameter fit on this table using AICc and the extra sum-of-squares F-test."*
- **Group and survival statistics:** *"Import experiment.csv, run a one-way ANOVA with Tukey post-hoc tests (or Kaplan–Meier survival analysis), and list significant differences."*
- **Publication figures:** *"Export every graph in study.curvebook as 300 DPI PNG and vector PDF."*
- **Existing `.pzfx` and `.prism` files:** *"Open old-assay.pzfx, list its tables, and run a t-test on the first one."* You can also export data tables back to `.pzfx` (data only; analyses and graphs stay in your `.curvebook` project).

---

## Troubleshooting

- **Tools do not appear:** Check that you edited the right config file, verify the command path and JSON punctuation, and **fully restart** your AI app.
- **"Access denied" error:** CurveBook blocks files outside your `--workdir` folder. Move your data inside that folder or update `--workdir`.
- **More connection checks:** See [Troubleshooting: MCP connection](reference/troubleshooting.md#mcp-connection).
- **Technical reference:** See the [MCP reference](reference/automation/mcp.md), [Control & batch automation](reference/automation/control-and-batch.md), and the [developer MCP guide](mcp.md).

---

## Your data stays private

- CurveBook collects **no telemetry** and never sends spreadsheets or projects to external servers.
- All calculations run offline inside your chosen workspace folder ([Privacy & Security](reference/privacy-security.md)).
- Your AI assistant sees only the tool calls and summaries in chat, never files outside `--workdir`.
