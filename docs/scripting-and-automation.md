# Scripting, Batch Pipelines, CLI & MCP Automation — Implementation Guidelines

Governing specification for headless batch workflows across `crates/engine` (`L5`), `crates/format` (`L3`), `crates/mcp` (`L6`), and `apps/lumencraft-cli` (`L7`). Related protocol specs: [`control-protocol.md`](control-protocol.md) and [`mcp.md`](mcp.md).

Public behavioural references:
- [Model Context Protocol (MCP) specification](https://modelcontextprotocol.io/)

---

## 1. Design Philosophy: Structured JSON Commands Over Legacy Text Macros

Traditional desktop biostatistics tools accumulated proprietary, imperative text-macro languages (`SetSheetTitle`, `ForEachFile`, `ImportCSV`, `ExportPDF`) that diverge from the interactive undo model and require a running GUI window.

LumenCraft replaces ad-hoc macro scripting with a **single unified, headless-capable command architecture**:
1. **Every operation is a registered `CommandSpec`** in `lumencraft-engine` (`crates/engine/src/cmd/*`, `cmd::registry()`) taking schema-validated JSON parameters and returning a structured `Result<CommandOutcome, EngineError>`.
2. **Four identical execution transports** share the exact same command registry and deterministic L0–L5 execution engine:
   - **Interactive GUI (`lumencraft` / `lumencraft-web`):** Menu items, dialogs, spreadsheet edits, and canvas tools dispatch commands onto `Session` (the drawn chrome is not yet fully human-operable; see `R10` in [ROADMAP.md](ROADMAP.md)).
   - **Reusable Method & Project Templates (`.lctemplate`):** Zero-code Assay / Plate repetition — open a template pre-wired with transforms, statistical tests, nonlinear fits, significance brackets, and page layouts, drop in new CSV/plate data, and every downstream result and figure updates deterministically.
   - **Headless Batch CLI (`lumencraft-cli`):** Run batch pipelines (`batch`, `run`, `convert`, `export`, `info`, `commands`, or any engine command ID directly) in CI, Snakemake, Nextflow, R, or Python without a display server (`xvfb` or GUI never required).
   - **Local Socket (`--control <port>`) & AI Agent Server (`lumencraft-cli mcp`):** Drive an in-memory headless project or a `lumencraft --control` session (which currently runs without a visible window) over JSONL / stdio MCP.

---

## 2. Batch Pipeline Recipes (`lumencraft-batch.json` / `.jsonl`)

For reproducible laboratory pipelines (e.g. processing 96-well / 384-well dose–response plates nightly), `lumencraft-cli batch <pipeline.json>` executes a deterministic sequence of engine commands with bounded file access rooted at an explicit `--workdir`:

```json
{
  "schema": "lumencraft.batch.v1",
  "template": "assay-4pl.lctemplate",
  "inputs": [
    { "sheet": "Dose Response", "csv": "plates/plate_01.csv" }
  ],
  "commands": [
    { "command": "info.setConstant", "params": { "sheet_id": "<info-sheet-id>", "name": "PlateID", "value": "P01" } }
  ],
  "exports": [
    { "format": "json", "sheet": "4PL Fit", "out": "out/plate_01_fit.json" },
    { "format": "csv", "sheet": "4PL Fit", "out": "out/plate_01_params.csv" },
    { "format": "pdf", "sheet": "Summary Layout", "out": "out/plate_01_figure.pdf" },
    { "out": "out/plate_01.lumencraft" }
  ]
}
```

### Batch Safety Invariants
- **Sandboxed path resolution:** Relative paths inside batch manifests resolve strictly beneath `--workdir`; `..` traversal escaping `--workdir` or absolute paths outside explicitly passed CLI flags return `CliError::PathEscape` (`apps/lumencraft-cli/src/sandbox.rs`, `WorkdirGuard` in `crates/platform/src/path_guard.rs`).
- **Deterministic output:** Given identical input CSVs and `.lctemplate`, `lumencraft-cli` emits bit-identical JSON/CSV result tables and deterministic SVG/PDF scene geometry across macOS, Linux, Windows, FreeBSD, and WASM.
- **Structured exit codes** (`apps/lumencraft-cli/src/error.rs`): `0` success, `1` runtime command/analysis failure (e.g. malformed CSV), `2` usage, manifest schema, or path-escape error, `130` interrupted — never silent partial exports on corrupt input.

---

## 3. Mapping Common Lab Automation Workflows

| Scientist Workflow | LumenCraft Mechanism |
|---|---|
| Repeat yesterday's analysis & graph on a new experiment | `File › New From Template…` (`project.instantiateTemplate`, `.lctemplate`) or `sheet.clonePipeline` ("Clone Pipeline") |
| Restyle 12 graphs in a manuscript to match Figure 1A | `graph.applyStyleFrom` ("Apply Style From…") across selected graph IDs |
| Fit 200 compound plates and export $\text{IC}_{50}$ + $95\%$ profile CIs to CSV | `lumencraft-cli batch` applying a `.lctemplate` over plate CSVs $\to$ tidy `results_csv` + multi-page `pdf` |
| Inspect, fit, diagnose & plot interactively from an AI research assistant | `lumencraft-cli mcp` (`doc_inspect`, `command_batch`, `render_preview`, `export`) |
| Drive a running LumenCraft session from external lab instrumentation | `lumencraft --control 7979` over `127.0.0.1` loopback JSONL ([`control-protocol.md`](control-protocol.md)) |
