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
   - **Headless Batch CLI (`lumencraft-cli`):** Run batch pipelines (`run`, `import`, `analyze`, `export`, `convert`, `verify`) in CI, Snakemake, Nextflow, R, or Python without a display server (`xvfb` or GUI never required).
   - **Live Socket (`--control <port>`) & AI Agent Server (`lumencraft-cli mcp`):** Drive either an in-memory headless project or a live desktop window over JSONL / stdio MCP.

---

## 2. Batch Pipeline Recipes (`lumencraft-batch.json` / `.jsonl`)

For reproducible laboratory pipelines (e.g. processing 96-well / 384-well dose–response plates nightly), `lumencraft-cli batch <pipeline.json>` executes a deterministic sequence of engine commands with bounded file access rooted at an explicit `--workdir`:

```json
{
  "schema": "lumencraft.batch.v1",
  "template": "assay-4pl.lctemplate",
  "inputs": [
    { "sheet": "Dose Response", "csv": "plates/plate_01.csv", "mode": "replace_data" }
  ],
  "commands": [
    { "command": "info.setConstant", "params": { "name": "PlateID", "value": "P01" } }
  ],
  "exports": [
    { "kind": "results_json", "sheet": "4PL Fit", "out": "out/plate_01_fit.json" },
    { "kind": "results_csv", "sheet": "4PL Fit", "out": "out/plate_01_params.csv" },
    { "kind": "pdf", "sheet": "Summary Layout", "out": "out/plate_01_figure.pdf" },
    { "kind": "project", "out": "out/plate_01.lumencraft" }
  ]
}
```

### Batch Safety Invariants
- **Sandboxed path resolution:** Relative paths inside batch manifests resolve strictly beneath `--workdir`; `..` traversal escaping `--workdir` or absolute paths outside explicitly passed CLI flags return `CliError::PathOutsideWorkdir`.
- **Deterministic output:** Given identical input CSVs and `.lctemplate`, `lumencraft-cli` emits bit-identical JSON/CSV result tables and deterministic SVG/PDF scene geometry across macOS, Linux, Windows, FreeBSD, and WASM.
- **Structured exit codes:** `0` success, `1` command/analysis failure (e.g. malformed CSV, schema mismatch), `2` CLI usage error — never silent partial exports on corrupt input.

---

## 3. Mapping Common Lab Automation Workflows

| Scientist Workflow | LumenCraft Mechanism |
|---|---|
| Repeat yesterday's analysis & graph on a new experiment | `File › New from Template…` (`.lctemplate`) or `sheet.clonePipeline` ("Clone Family") |
| Restyle 12 graphs in a manuscript to match Figure 1A | `graph.applyStyleFrom` ("Apply Graph Style / Wand") across selected graph IDs |
| Fit 200 compound plates and export $\text{IC}_{50}$ + $95\%$ profile CIs to CSV | `lumencraft-cli batch` applying a `.lctemplate` over plate CSVs $\to$ tidy `results_csv` + multi-page `pdf` |
| Inspect, fit, diagnose & plot interactively from an AI research assistant | `lumencraft-cli mcp` (`doc_inspect`, `command_batch`, `render_preview`, `export`) |
| Drive a live open desktop window from external lab instrumentation | `lumencraft --control 7979` over `127.0.0.1` loopback JSONL ([`control-protocol.md`](control-protocol.md)) |
