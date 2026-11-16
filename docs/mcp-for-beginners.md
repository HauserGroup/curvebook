# CurveBook: a beginner's guide

**Analyze your data and make scientific graphs by asking your AI assistant.**

For example:

> Open my results, compare the two groups, and make a graph.

Your AI assistant passes the work to CurveBook, which performs the calculations and creates the results.

The connection between the two programs is called **MCP**. You only need to set it up once. After that, you work by typing ordinary requests.

**CurveBook is an early version. Its AI connection works; its point-and-click interface is still being developed.**

## Before you start

You need:

- **An AI application that supports local MCP tools.** The instructions below use Claude Desktop.
- **The CurveBook helper program**, named `curvebook-cli` on Mac or `curvebook-cli.exe` on Windows.
- **A folder for your data and saved work.**

The helper program is the part of CurveBook your AI assistant uses. You do not normally open it yourself.

### Getting CurveBook

Check the [Downloads page](https://github.com/HauserGroup/curvebook/releases) for a package for your computer. Download a ready-to-use package from the release's **Assets** list, rather than "Source code."

If no download is available yet, someone comfortable building software will need to prepare CurveBook for you. The [developer instructions](development.md) explain how.

The setup below starts once you have the helper program.

## 1. Make a folder for CurveBook

1. Open your **Documents** folder.
2. Create a folder called **CurveBook**.
3. Put the helper program inside it.
4. Inside that folder, create another folder called **Data**.

Your folders should look like this:

```text
Documents
└── CurveBook
    ├── curvebook-cli       ← curvebook-cli.exe on Windows
    └── Data
```

Put copies of the files you want to analyze inside **Data**. CurveBook will also save your projects and exported graphs there.

Keep the helper program in this location after setup. Moving it will break the connection.

## 2. Connect your AI assistant

These instructions use the **Claude desktop application**, rather than Claude in a web browser.

Open Claude Desktop's application settings and find **Local MCP servers**. Depending on your version, this appears under **Desktop app → Extensions → Manage**, or under **Developer** in the older layout. [Claude's setup guide](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop)

Open **Edit Config**. This opens a settings file containing text.

If the file is empty, paste the example for your computer below. **If it already contains settings for other tools, keep those settings—ask for help adding CurveBook alongside them.**

### On a Mac

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "/Users/YOURNAME/Documents/CurveBook/curvebook-cli",
      "args": [
        "mcp",
        "--workdir",
        "/Users/YOURNAME/Documents/CurveBook/Data"
      ]
    }
  }
}
```

### On Windows

```json
{
  "mcpServers": {
    "curvebook": {
      "command": "C:\\Users\\YOURNAME\\Documents\\CurveBook\\curvebook-cli.exe",
      "args": [
        "mcp",
        "--workdir",
        "C:\\Users\\YOURNAME\\Documents\\CurveBook\\Data"
      ]
    }
  }
}
```

**Replace `YOURNAME` in both places with your computer's account-folder name.** Keep the quotation marks, commas, and other punctuation as shown.

These examples assume your folders are in the locations shown. If Documents is stored somewhere else, such as OneDrive, use its actual location.

Save the settings file, completely quit Claude Desktop, and open it again. Claude should now start the CurveBook helper automatically. [Manual MCP configuration instructions](https://modelcontextprotocol.io/docs/develop/connect-local-servers)

## 3. Check that it works

Start a new chat and paste this:

> Use the CurveBook tools to compare these two independent groups:
>
> Control: 10, 11, 12
>
> Treated: 20, 22, 24
>
> Run Welch's t-test, explain the result in simple language, and save the project as `first-example.curvebook`.

Your assistant should use CurveBook's tools and explain the results. It may ask permission before using a tool.

Look inside your **Data** folder for `first-example.curvebook`. That confirms the connection can both perform an analysis and save your work.

**It is normal for no CurveBook window to appear.** The helper works in the background.

## 4. Analyze your own data

Copy a CSV file into **Data**. CSV is a common spreadsheet format; most spreadsheet applications can save a sheet as CSV.

Then ask:

> Use CurveBook to open `results.csv`. Show me the column names and a few example rows. Ask what I want to find out before choosing an analysis.

You can then ask things such as:

- "Explain what these results mean."
- "Make a graph comparing these groups."
- "Fit a dose-response curve to these measurements."
- "Save the graph as `comparison.pdf`."

Tell your assistant what the measurements represent, whether observations are paired, and what question you want to answer.

**Before finishing, ask it to save your project:**

> Save my work as `my-analysis.curvebook`.

A saved CurveBook project lets you continue later. To reopen it, ask:

> Open `my-analysis.curvebook` using CurveBook.

## If something does not work

| Problem | What to try |
|---|---|
| Your assistant cannot find CurveBook | Completely quit and reopen the AI application, then start a new chat. |
| The connection fails to start | Check that the helper program exists at the location entered in the settings. |
| CurveBook cannot find your data | Put the file inside **Data** and check its exact filename. |
| A file-access error appears | Check that the **Data** folder exists and that you have permission to use it. |
| No CurveBook window opens | This is expected when using the AI connection. |

For help, report your operating system, AI application, and exact error message through [GitHub Issues](https://github.com/HauserGroup/curvebook/issues). Avoid attaching private research data.

## Privacy

CurveBook performs its calculations on your computer and collects no telemetry.

Your AI application may send conversation content and tool results to its provider. Its privacy settings still apply to the data you use in a chat.

## Technical documentation

For other MCP applications, advanced settings, and developer setup, see the [MCP guide](mcp.md) and [development guide](development.md).

[Back to the README](../README.md)
