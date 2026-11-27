# Using CurveBook with your AI assistant

<!-- Audience: Tier 1 Scientist Guide — keep sentences short and plain; put full MCP tool schemas, flags, and protocol details in docs/reference/automation/mcp.md. -->

CurveBook includes a [desktop app](reference/desktop-gui.md) and a built-in **MCP (Model Context Protocol)** server. You can ask an AI assistant in plain English to run analyses, then polish the saved `.curvebook` project in the desktop app.

Every desktop installer includes both the app and `curvebook-cli`. CurveBook itself runs offline on your computer and collects no telemetry.

> **Without an AI assistant:** Follow [Get started](reference/get-started.md) to install the desktop app, then read the [desktop app overview](reference/desktop-gui.md).

---

## What you need

1. **CurveBook**, installed from the [Releases page](https://github.com/HauserGroup/curvebook/releases). Follow [Get started](reference/get-started.md) to install it and verify your download.
2. **A desktop AI app that supports MCP**, such as Claude Desktop, ChatGPT Work (in the ChatGPT desktop app), Codex, Cursor, or Windsurf.
3. **A folder for your data**, such as `Documents/MyResearchData`, to hold your CSV files and `.curvebook` projects.

---

## Step 1: Find curvebook-cli on your computer

Each installer places `curvebook-cli` next to the desktop app:

- **macOS:** Once **CurveBook.app** is in **Applications**, the command-line tool is at:
  ```text
  /Applications/CurveBook.app/Contents/MacOS/curvebook-cli
  ```
- **Windows:** For the default per-user install, the command-line tool is at:
  ```text
  %LocalAppData%\Programs\CurveBook\curvebook-cli.exe
  ```
  For a machine-wide install, it is at `%ProgramFiles%\CurveBook\curvebook-cli.exe`.
- **Linux:** Make the AppImage executable and extract it once, so your AI app need not unpack it every time:
  ```sh
  chmod +x curvebook-<version>-linux-x86_64.AppImage
  ./curvebook-<version>-linux-x86_64.AppImage --appimage-extract
  ```
  Use the `aarch64` file on ARM64 Linux. The command-line tool is at:
  ```text
  <path-to-extracted-folder>/squashfs-root/usr/bin/curvebook-cli
  ```
- **FreeBSD and source builds:** See [FreeBSD install](reference/get-started.md#install-freebsd) or the [developer MCP guide](mcp.md).

---

## Step 2: Connect CurveBook to your AI app (one-time setup)

Open your AI assistant's MCP configuration file:

- **Claude Desktop (macOS):** `~/Library/Application Support/Claude/claude_desktop_config.json` (or use **Settings › Developer › Edit Config**).
- **Claude Desktop (Windows):** `%APPDATA%\Claude\claude_desktop_config.json` (or use **Settings › Developer › Edit Config**).
- **ChatGPT Work and Codex:** These apps use a TOML file instead of JSON. See [ChatGPT Work and Codex](#chatgpt-work-and-codex) below.
- **Other MCP clients (Cursor, Windsurf, Cline):** Open your client's MCP server settings or its `mcp.json` file.

Add a `curvebook` entry for your operating system. Replace `YOURNAME` and the `--workdir` path with your own username and data folder.

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

> **Windows tip:** Inside JSON strings, write each backslash twice (`\\`). Type your real username instead of `%LocalAppData%`, because JSON config files do not expand environment variables.

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

Save the file, then quit and reopen your AI app so it loads CurveBook.

### ChatGPT Work and Codex {#chatgpt-work-and-codex}

ChatGPT Work (in the ChatGPT desktop app), the Codex CLI, and the Codex IDE extension share one settings file: `~/.codex/config.toml` (or `%USERPROFILE%\.codex\config.toml` on Windows). Add CurveBook once to use it in all three.

Add CurveBook in the app's MCP server settings, or run this once in a terminal (macOS example):

```sh
codex mcp add curvebook -- /Applications/CurveBook.app/Contents/MacOS/curvebook-cli mcp --workdir /Users/YOURNAME/Documents/MyResearchData
```

Alternatively, add this block to `config.toml`:

```toml
[mcp_servers.curvebook]
command = "/Applications/CurveBook.app/Contents/MacOS/curvebook-cli"
args = ["mcp", "--workdir", "/Users/YOURNAME/Documents/MyResearchData"]
```

On Windows and Linux, use the paths from the JSON examples above. In Windows TOML files, single quotes let you use single backslashes, as in `command = 'C:\Users\YOURNAME\...'`.

To load CurveBook, restart the ChatGPT desktop app or start a new Codex session. ChatGPT in a web browser cannot start programs on your computer, so use the desktop app.

> **About `--workdir`:** This flag limits CurveBook to that folder and its subfolders. CurveBook will not read or write files outside it.

---

## Step 3: Run your first analysis in plain English

Start a new chat in your AI app and paste this prompt:

> Use CurveBook to create a Column table comparing two groups:
> - Control: 10.2, 9.8, 10.5, 10.0, 10.1
> - Treated: 14.1, 13.8, 14.5, 13.9, 14.2
>
> Run a Welch's t test and show the summary statistics and P value. Create a box-and-whisker graph with individual points, export comparison.png and comparison.pdf, and save first-example.curvebook.

CurveBook runs the analysis on your computer and returns the *t*-test summary and *P* value. It saves `comparison.png`, `comparison.pdf`, and `first-example.curvebook` in your data folder.

---

## Step 4: Open the project in the desktop app

CurveBook saves standard `.curvebook` files, which you can open directly in the desktop app:

1. Open **CurveBook** from **Applications** (macOS), the Start Menu (Windows), or your AppImage (Linux).
2. Choose **File › Open…** (`Cmd/Ctrl+O`), or drag `first-example.curvebook` onto the CurveBook window.
3. Browse your work with the **Project Navigator** and the **Family** tree on the left. Open the **Data** table to check values, **Results** to inspect diagnostics, or **Graph** to polish axes, error bars, and layouts.

This also works in reverse. Edit and save a project in the desktop app, then ask your AI assistant to run follow-up fits or batch exports.

---

## Everyday things you can ask for

- **Curve fitting:** *"Fit a four-parameter logistic dose-response curve to the XY table Dose response and report the EC50 with its 95% confidence interval."*
- **Model comparison:** *"Fit three-parameter and four-parameter logistic curves to the XY table Dose response and compare them with AICc and the extra sum-of-squares F test."*
- **Group comparisons:** *"Import experiment.csv as a Column table, run a one-way ANOVA with Tukey's multiple comparisons, and list the significant differences."*
- **Survival analysis:** *"Import survival.csv as a Survival table, plot Kaplan–Meier curves for each group, and compare them with a log-rank test."*
- **Publication figures:** *"Export every graph in study.curvebook as 300 DPI PNG and vector PDF."*
- **Existing `.pzfx` and `.prism` files:** *"Open old-assay.pzfx, list its tables, and run an unpaired t test on the first one."* You can also export data tables back to `.pzfx`. Only data is exported; analyses and graphs stay in your `.curvebook` project.

---

## Troubleshooting

- **Tools do not appear:** Check that you edited the right config file and that the command path and JSON punctuation are correct. Then fully quit and reopen your AI app.
- **"Access denied" error:** CurveBook blocks access to files outside your `--workdir` folder. Move your data into that folder, or change `--workdir`.
- **More connection checks:** See [Troubleshooting: MCP connection](reference/troubleshooting.md#mcp-connection).
- **Technical reference:** See the [MCP reference](reference/automation/mcp.md), [control and batch automation](reference/automation/control-and-batch.md), and the [developer MCP guide](mcp.md).

---

## Your data stays private

- CurveBook collects no telemetry and never sends your spreadsheets or projects to external servers.
- All calculations run offline, and CurveBook works only inside your chosen data folder ([Privacy and Security](reference/privacy-security.md)).
- CurveBook cannot read or write files outside `--workdir`.
- What your AI app does with the results it receives is governed by that app's own privacy policy ([Privacy Policy](privacy.html)).
