# Scripting, Batch Pipelines, CLI & MCP Automation — Implementation Guidelines

Governing specification for headless batch workflows across `crates/engine` (`L5`), `crates/format` (`L3`), `crates/mcp` (`L6`), and `apps/curvebook-cli` (`L7`). Related protocol specs: [`control-protocol.md`](../control-protocol.md) and [`mcp.md`](../mcp.md).

Public behavioural references:
- [Model Context Protocol (MCP) specification](https://modelcontextprotocol.io/)

---

## 1. Design Philosophy: Structured JSON Commands Over Legacy Text Macros

Traditional desktop biostatistics tools accumulated proprietary, imperative text-macro languages (`SetSheetTitle`, `ForEachFile`, `ImportCSV`, `ExportPDF`) that diverge from the interactive undo model and require a running GUI window.

CurveBook replaces ad-hoc macro scripting with a **single unified, headless-capable command architecture**:
1. **Every operation is a registered `CommandSpec`** in `curvebook-engine` (`crates/engine/src/cmd/*`, `cmd::registry()`) taking schema-validated JSON parameters and returning a structured `Result<CommandOutcome, EngineError>`.
2. **Four identical execution transports** share the exact same command registry and deterministic L0–L5 execution engine:
   - **Interactive GUI (`curvebook` / `curvebook-web`):** Menu items, dialogs, spreadsheet edits, and canvas tools in the `egui 0.37` desktop shell (`apps/curvebook`, `crates/ui-egui`, `65 / 65` five-stage human-path workflows complete) dispatch commands onto `Session`; `apps/curvebook-web` provides a strict-CSP `wasm32` bridge pending full `eframe::WebRunner` adoption in [`R11.11`](../ROADMAP.md#r1111-migrate-browser-host-to-eframe-webrunner-10-gate).
   - **Reusable Method & Project Templates (`.cbtemplate`):** Zero-code Assay / Plate repetition — open a template pre-wired with transforms, statistical tests, nonlinear fits, significance brackets, and page layouts, drop in new CSV/plate data, and every downstream result and figure updates deterministically.
   - **Headless Batch CLI (`curvebook-cli`):** Run batch pipelines and inspect, convert, or export projects headlessly via `run`, `batch`, `convert`, `export`, `info`, `commands`, direct `<command-id>` dispatch (for example `table.importCsv`, `stats.run`, `fit.run`), `bench`, `perf`, and `mcp` in CI, Snakemake, Nextflow, R, or Python without a display server (`xvfb` or GUI never required).
   - **Live Socket (`--control <port>`) & AI Agent Server (`curvebook-cli mcp`):** Drive an in-memory headless project, a headless `curvebook --control` session, or a live desktop window over JSONL / stdio MCP.

---

## 2. Batch Pipeline Recipes (`curvebook.batch.v1` / `.jsonl`)

For reproducible laboratory pipelines (e.g. processing 96-well / 384-well dose–response plates nightly), `curvebook-cli batch <manifest.json> [--workdir <dir>] [--json]` executes a deterministic `curvebook.batch.v1` manifest ([`schemas/batch-v1.schema.json`](../schemas/batch-v1.schema.json)) and `curvebook-cli run <script.jsonl> [--workdir <dir>] [--json]` executes a JSON-lines or JSON-array command stream, both with bounded file access rooted at an explicit `--workdir`:

```json
{
  "schema": "curvebook.batch.v1",
  "template": "assay-4pl.cbtemplate",
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
    { "format": "curvebook", "out": "out/plate_01.curvebook" }
  ]
}
```

### Manifest Schema Summary (`curvebook.batch.v1`)
- **Top-level:** `schema` (required `"curvebook.batch.v1"`), optional `template` or `project` (mutually exclusive), optional `inputs`, `commands`, `exports`. Unknown fields fail validation with `CliError::Schema` (exit code `2`). Legacy `lumencraft.batch.v1` manifests are explicitly rejected.
- **`inputs[]`:** `csv` (or `tsv`, `path`, `file`), optional `sheet` (sheet ID or title; matching an existing data sheet by ID, exact title, or ASCII case-insensitive title replaces that sheet in place, while an unmatched or omitted `sheet` creates a new sheet), optional `kind` (table archetype for new sheets, defaulting to `"xy"`), `subcolumn_mode`, `delimiter`, `decimal_separator`.
- **`commands[]`:** `cmd` (or `command`, `method`), optional `id`, optional `params` (JSON object).
- **`exports[]`:** `out` (or `path`), optional `format` (`"curvebook"`, `"curvebook.d"`, `"svg"`, `"pdf"`, `"emf"`, `"png"`, `"tiff"`, `"csv"`, `"tsv"`, `"json"`; inferred from `out` extension or `.curvebook.d` suffix when omitted), optional `sheet` (ID or title), `dpi` (positive integer for `png`/`tiff`), `pretty` (boolean for `json`, default `true`), `tab_index` (0-based `ResultSheet` tab index for `csv`/`tsv`), `pdf_version` (`"1.4"` or `"1.7"`, default `"1.7"`), `ttf_font` (TrueType `.ttf` path for `pdf`).

### Batch Safety Invariants
- **Sandboxed path resolution:** Under `--workdir <dir>`, paths inside `run` scripts and `batch` manifests (including `template`, `project`, `inputs[].csv`/`path`, `exports[].out`/`ttf_font`, command path parameters, and stored `ExternalDelimitedFile` link refreshes) resolve strictly beneath the canonicalized `--workdir`; `..` traversal, escaping symlinks, or paths outside `--workdir` return `CliError::PathEscape` (`apps/curvebook-cli/src/sandbox.rs`, `WorkdirGuard` in `crates/platform/src/path_guard.rs`; exit code `2`; `--json` error `code: "path_escape"`, `category: "policy"`).
- **Deterministic output:** Given identical input CSVs and `.cbtemplate`, `curvebook-cli` emits bit-identical JSON/CSV result tables and deterministic SVG/PDF scene geometry across macOS, Linux, Windows, FreeBSD, and WASM.
- **Structured exit codes** (`apps/curvebook-cli/src/error.rs`): `0` (`EXIT_OK`) success, `1` (`EXIT_RUNTIME`) runtime/I/O/data/format/analysis/rendering failure (e.g. malformed CSV), `2` (`EXIT_USAGE_OR_POLICY`) CLI usage error, invalid script/manifest schema (`CliError::Schema`), or `--workdir` path escape (`CliError::PathEscape`), `130` (`EXIT_INTERRUPTED`) cancellation/interruption (`CliError::Interrupted`) — never silent partial exports on corrupt input.

---

## 3. Mapping Common Lab Automation Workflows

| Scientist Workflow | CurveBook Mechanism |
|---|---|
| Repeat yesterday's analysis & graph on a new experiment | `File › New From Template…` (`project.instantiateTemplate`, `.cbtemplate`) or `sheet.clonePipeline` ("Clone Pipeline" / "Clone Family") |
| Restyle 12 graphs in a manuscript to match Figure 1A | `graph.applyStyleFrom` ("Apply Graph Style / Wand") across selected graph IDs |
| Fit 200 compound plates and export $\text{IC}_{50}$ + $95\%$ profile CIs to CSV | `curvebook-cli batch` applying a `.cbtemplate` over plate CSVs $\to$ tidy `csv` result tables + multi-page `pdf` |
| Inspect, fit, diagnose & plot interactively from an AI research assistant | `curvebook-cli mcp` (`doc_inspect`, `command_batch`, `render_preview`, `export`) |
| Drive a running CurveBook session from external lab instrumentation | `curvebook --control 7979` over `127.0.0.1` loopback JSONL ([`control-protocol.md`](../control-protocol.md)) |
