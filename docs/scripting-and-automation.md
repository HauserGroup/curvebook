# Scripting, Batch Pipelines, CLI & MCP Automation — Implementation Guidelines

Governing specification for headless batch workflows across `crates/engine` (`L5`), `crates/format` (`L3`), `crates/mcp` (`L6`), and `apps/lumencraft-cli` (`L7`). Related protocol specs: [`control-protocol.md`](control-protocol.md) and [`mcp.md`](mcp.md).

Public behavioural references:
- [Automating repetitive work: Templates, Wand & batch scripts](https://www.graphpad.com/guides/prism/latest/user-guide/automating_analyses.htm)
- [Model Context Protocol (MCP) specification](https://modelcontextprotocol.io/)

---

## 1. Design Philosophy: Structured JSON Commands Over Legacy Text Macros

Traditional desktop biostatistics tools accumulated proprietary, imperative text-macro languages (`SetSheetTitle`, `ForEachFile`, `ImportCSV`, `ExportPDF`) that diverge from the interactive undo model and require a running GUI window.

LumenCraft replaces ad-hoc macro scripting with a **single unified, headless-capable command architecture**:
1. **Every operation is an `EngineCommand`** in `lumencraft-engine` (`crates/engine/src/cmd/*`) taking validated JSON parameters and returning structured JSON results (`Result<Value, EngineError>`).
2. **Four identical execution transports** share the exact same command registry and deterministic L0–L5 execution engine:
   - **Interactive GUI (`lumencraft` / `lumencraft-web`):** Menu items, dialogs, spreadsheet edits, and canvas tools dispatch commands onto `Session`.
   - **Reusable Method & Project Templates (`.lctemplate`):** Zero-code Assay / Plate repetition — open a template pre-wired with transforms, statistical tests, nonlinear fits, significance brackets, and page layouts, drop in new CSV/plate data, and every downstream result and figure updates deterministically.
   - **Headless Batch CLI (`lumencraft-cli`):** Run batch pipelines and inspect, convert, or export projects headlessly via `run`, `batch`, `convert`, `export`, `info`, `commands`, direct `<command-id>` dispatch (for example `table.importCsv`, `stats.run`, `fit.run`), `bench`, `perf`, and `mcp` in CI, Snakemake, Nextflow, R, or Python without a display server (`xvfb` or GUI never required).
   - **Live Socket (`--control <port>`) & AI Agent Server (`lumencraft-cli mcp`):** Drive either an in-memory headless project or a live desktop window over JSONL / stdio MCP.

---

## 2. Batch Pipeline Recipes (`lumencraft.batch.v1` / `.jsonl`)

For reproducible laboratory pipelines (e.g. processing 96-well / 384-well dose–response plates nightly), `lumencraft-cli batch <manifest.json> [--workdir <dir>] [--json]` executes a deterministic `lumencraft.batch.v1` manifest ([`schemas/batch-v1.schema.json`](schemas/batch-v1.schema.json)) and `lumencraft-cli run <script.jsonl> [--workdir <dir>] [--json]` executes a JSON-lines or JSON-array command stream, both with bounded file access rooted at an explicit `--workdir`:

```json
{
  "schema": "lumencraft.batch.v1",
  "template": "assay-4pl.lctemplate",
  "inputs": [
    { "sheet": "Dose Response", "csv": "plates/plate_01.csv", "kind": "xy" }
  ],
  "commands": [
    { "cmd": "info.setConstant", "params": { "name": "PlateID", "value": "P01" } }
  ],
  "exports": [
    { "format": "json", "sheet": "4PL Fit", "out": "out/plate_01_fit.json" },
    { "format": "csv", "sheet": "4PL Fit", "out": "out/plate_01_params.csv", "tab_index": 0 },
    { "format": "pdf", "sheet": "Summary Layout", "out": "out/plate_01_figure.pdf" },
    { "format": "lumencraft", "out": "out/plate_01.lumencraft" }
  ]
}
```

### Manifest Schema Summary (`lumencraft.batch.v1`)
- **Top-level:** `schema` (required `"lumencraft.batch.v1"`), optional `template` or `project` (mutually exclusive), optional `inputs`, `commands`, `exports`. Unknown fields fail validation with `CliError::Schema` (exit code `2`).
- **`inputs[]`:** `csv` (or `tsv`, `path`, `file`), optional `sheet` (sheet ID or title; matching an existing data sheet by ID, exact title, or ASCII case-insensitive title replaces that sheet in place, while an unmatched or omitted `sheet` creates a new sheet), optional `kind` (table archetype for new sheets, defaulting to `"xy"`), `subcolumn_mode`, `delimiter`, `decimal_separator`.
- **`commands[]`:** `cmd` (or `command`, `method`), optional `id`, optional `params` (JSON object).
- **`exports[]`:** `out` (or `path`), optional `format` (`"lumencraft"`, `"lumencraft.d"`, `"svg"`, `"pdf"`, `"emf"`, `"png"`, `"tiff"`, `"csv"`, `"tsv"`, `"json"`; inferred from `out` extension or `.lumencraft.d` suffix when omitted), optional `sheet` (ID or title), `dpi` (positive integer for `png`/`tiff`), `pretty` (boolean for `json`, default `true`), `tab_index` (0-based `ResultSheet` tab index for `csv`/`tsv`), `pdf_version` (`"1.4"` or `"1.7"`, default `"1.7"`), `ttf_font` (TrueType `.ttf` path for `pdf`).

### Batch Safety Invariants
- **Sandboxed path resolution:** Under `--workdir <dir>`, paths inside `run` scripts and `batch` manifests (including `template`, `project`, `inputs[].csv`/`path`, `exports[].out`/`ttf_font`, command path parameters, and stored `ExternalDelimitedFile` link refreshes) resolve strictly beneath the canonicalized `--workdir`; `..` traversal, escaping symlinks, or paths outside `--workdir` return `CliError::PathEscape` (exit code `2`; `--json` error `code: "path_escape"`, `category: "policy"`).
- **Deterministic output:** Given identical input CSVs and `.lctemplate`, `lumencraft-cli` emits bit-identical JSON/CSV result tables and deterministic SVG/PDF scene geometry across macOS, Linux, Windows, FreeBSD, and WASM.
- **Structured exit codes:** `0` (`EXIT_OK`) success, `1` (`EXIT_RUNTIME`) runtime/I/O/data/format/analysis/rendering failure, `2` (`EXIT_USAGE_OR_POLICY`) CLI usage error, invalid script/manifest schema (`CliError::Schema`), or `--workdir` path escape (`CliError::PathEscape`), `130` (`EXIT_INTERRUPTED`) cancellation/interruption (`CliError::Interrupted`) — never silent partial exports on corrupt input.

---

## 3. Mapping Common Lab Automation Workflows

| Scientist Workflow | LumenCraft Mechanism |
|---|---|
| Repeat yesterday's analysis & graph on a new experiment | `File › New from Template…` (`.lctemplate`) or `sheet.clonePipeline` ("Clone Family") |
| Restyle 12 graphs in a manuscript to match Figure 1A | `graph.applyStyleFrom` ("Apply Graph Style / Wand") across selected graph IDs |
| Fit 200 compound plates and export $\text{IC}_{50}$ + $95\%$ profile CIs to CSV | `lumencraft-cli batch` applying a `.lctemplate` over plate CSVs $\to$ `csv` result tables + multi-page `pdf` |
| Inspect, fit, diagnose & plot interactively from an AI research assistant | `lumencraft-cli mcp` (`doc_inspect`, `command_batch`, `render_preview`, `export`) |
| Drive a live open desktop window from external lab instrumentation | `lumencraft --control 7979` over `127.0.0.1` loopback JSONL ([`control-protocol.md`](control-protocol.md)) |
