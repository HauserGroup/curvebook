# R17.4 Live-Stdio MCP Usability Trials & Ergonomics Evidence (`curvebook-mcp`, `curvebook-cli`)

> Scope: Headless & `--workdir` stdio JSON-RPC trials against the real `curvebook-cli mcp` subprocess across four end-to-end scientific workflows, schema/response token compaction, actionable remediation hints, and deterministic regression tests.

## 1. Trial Methodology

All quantitative measurements below were collected against real `curvebook-cli mcp --workdir <dir>` subprocesses using Content-Length framed JSON-RPC 2.0 over stdin/stdout (excluding base64 PNG image payloads from text-token estimates; token estimates use `ceil(non_image_wire_bytes / 4)`).

- **Subprocess workflows evaluated:**
  1. **Discovery & schema surface (`discovery`, 11 calls):** `initialize`, `tools/list` (core 10), `list_toolsets`, `command_list({})`, `command_list({ filter: "fit", include_ui: false })`, `prompts/list`, 3× `prompts/get`, `enable_toolset("all")`, and `tools/list` (all enabled).
  2. **Workflow 1 — Messy tabular import & cleanup (`wf1_import`, 9 calls):** Semicolon-delimited ELISA CSV with empty trailing column, `"ND"` placeholder, and `"9999.0"` spike (`open_file`), root & single-sheet `doc_inspect`, `sheet://` read, outlier exclusion with `table.excludeCellsWithReason`, rename, and CSV with two non-numeric preamble header lines appended.
  3. **Workflow 2 — Statistical group comparison (`wf2_stats`, 12 calls):** `prompts/get` (`choose_two_group_test`), 3-group Column table creation, erroneous guessed kind (`"anova_one_way"`), `stats.run` (`one_way_anova`, `kruskal_wallis`), root & sheet `doc_inspect`, and `result://` read.
  4. **Workflow 3 — Nonlinear dose-response fit with outlier & identifiability diagnostics (`wf3_fit`, 11 calls):** 8-dose × 3-replicate XY table with an injected outlier (`195.0`), OLS (`identify_unstable_parameters: true`) and ROUT (`method: "rout"`, `identify_unstable_parameters: true`) fits on `log_inhibitor_vs_response_4pl`, root & sheet `doc_inspect`, and `result://` read.
  5. **Workflow 4 — Publication figure & export (`wf4_pub`, 14 calls):** `prompts/get` (`publication_figure_checklist`), XY dose-response setup, 4PL fit, `graph.setRegressionOverlay`, `graph.preflightCheck`, graph `doc_inspect(include_cells: false)`, `render_preview`, `export` (`.svg`, `.pdf`, `.png`), and root `doc_inspect`.

> **External interactive boundary:** Interactive LLM client trials inside external GUI MCP hosts (such as Claude Desktop, Cursor, and Codex GUI integrations) remain external and were not driven via GUI automation here. Local measurements reflect scripted realistic agent JSON-RPC sequences against the live `curvebook-cli mcp` stdio server.

---

## 2. Baseline vs. After Measurements

### 2.1 Apples-to-Apples Identical Query Comparison (Default Parameters)

Every row runs the exact same JSON-RPC call sequence before and after R17.4 fixes:

| Stage / Operation | Calls | Baseline Wire (B) | Baseline Est. Tokens | After Wire (B) | After Est. Tokens | Reduction |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `discovery` (11 calls) | 11 | 994,383 | 248,601 | 307,479 | 76,873 | **-69.1%** |
| `wf1_import` (9 calls) | 9 | 8,354 | 2,091 | 8,197 | 2,053 | -1.9% *(added messy-CSV warnings)* |
| `wf2_stats` (12 calls) | 12 | 56,245 | 14,067 | 29,417 | 7,358 | **-47.7%** |
| `wf3_fit` (11 calls) | 11 | 183,530 | 45,888 | 89,561 | 22,395 | **-51.2%** |
| `wf4_pub` (14 non-img JSON) | 14 | 32,084 | 8,026 | 16,760 | 4,194 | **-47.8%** |

### 2.2 High-Leverage Single-Call & Targeted Query Savings

| Call | Baseline Wire (B) | After (Identical Call) | After (With R17.4 Targeted Option) | Peak Reduction |
| --- | ---: | ---: | ---: | ---: |
| `command_list({})` | 675,921 (~168,981 tok) | 75,568 (~18,892 tok) | — | **-88.8%** |
| `command_list({ filter: "fit", include_ui: false })` | 26,282 (~6,571 tok) | 1,884 (~471 tok) | — | **-92.8%** |
| `list_toolsets({})` | 90,153 (~22,539 tok) | 40,024 (~10,006 tok) | `include_tools: false` → 1,972 (~493 tok) | **-97.8%** |
| `doc_inspect({})` (2 analyses, WF3) | 25,378 (~6,345 tok) | 10,732 (~2,683 tok) | `detail: "compact"` → 4,207; `sections: ["analyses"]` → 1,515 | **-94.0%** |
| `doc_inspect({ sheet_id: "fit-rout", include_cells: true })` | 46,479 (~11,620 tok) | 12,629 (~3,158 tok) | `tab_id: "nonlinear_parameters"` → 5,299 (~1,325 tok) | **-88.6%** |
| `doc_inspect({ sheet_id: "pub-graph", include_cells: false })` | 6,552 (~1,638 tok) | 868 (~217 tok) | — | **-86.8%** |
| `resources/read` (`result://fit-rout`) | 98,657 (~24,665 tok) | 55,053 (~13,764 tok) | `?include_curves=false` → 15,934 (~3,984 tok) | **-83.9%** |
| `tools/list` (all 105 engine tools enabled) | 187,387 (~46,847 tok) | 172,229 (~43,058 tok) | — | **-8.1%** |

---

## 3. Friction Points Discovered & Fixes Implemented

1. **Redundant Pretty-Print Whitespace Across Tools & Resources (`crates/mcp/src/{tools.rs, errors.rs, resources.rs}`):**
   - `ToolCallResult::ok_json`, `errors::result_from_payload_with_extra`, `read_sheet_resource`, and `read_result_resource` pretty-printed JSON into `content[0].text` while also sending `structuredContent`, inflating text tokens by ~35–48%. Switched all MCP JSON text encodings to compact single-line JSON (`serde_json::to_string` and `export_result_sheet_json(rs, false)`).
2. **`command_list` & `list_toolsets` Catalog Bloat (`crates/mcp/src/{tools.rs, toolsets.rs}`):**
   - `command_list({})` unconditionally serialized all 105 engine parameter JSON schemas and 53 UI specs (~676 KB). Changed `command_list` to return concise summaries by default and added `command_id` (matching both dot IDs and `mcp_tool` names, defaulting `include_schemas` to `true`), `include_schemas`, and `detail` (`summary` | `full`). Normalized `_`/`.` in substring filters.
   - `list_toolsets` now omits per-engine-command annotations and duplicate label/URL strings in default `compact` mode and supports `include_tools`, `group`, and `detail` while rejecting unknown arguments.
   - Added `compact_json_schema` in `toolsets.rs` (applied in `command_spec_to_tool_definition`, `command_list`, and `gui_tools.rs`) to strip redundant `$schema` URIs and collapse `anyOf: [<type>, {"type":"null"}]` wrappers into `"type": ["<type>", "null"]`.
3. **`doc_inspect` Duplication, Eager `GraphSpec` Fetch, & Lack of Scoping (`crates/mcp/src/tools.rs`):**
   - Root summary duplicated `analyses`/`graphs`/`layouts` inside `sheets`, called `inspect_sheet_detail(doc, &rs.id, true)` only to discard tabs/checklists, and repeated empty/null metadata (`units: null`, `hooks: []`, `upstream_dependencies: []`). Eliminated duplicate `sheets.{analyses,graphs,layouts}` (preserving `/sheets/data`, `/sheets/info`, `analyses`, `graphs`, `layouts`, `variables.result_scalars`, and `file_path`), computed `analyses` summaries directly, and added `detail` (`compact` | `standard` | `full`), `sections`, `tab_id`, `max_rows`, and `max_scalars`.
   - Fixed `inspect_single_sheet` so `include_cells: false` no longer fetches full `GraphSpec`/`LayoutSpec` blobs unless `detail == "full"`, returning concise graph metadata (`kind`, `primary_source_sheet_id`, `series_summary`, counts) instead.
   - Compacted `ResultTab` cells inside `doc_inspect` (bare numbers/booleans/strings, `{"ci":[lo,hi]}`, `{"p":raw,"formatted":...,"stars":...}`) and exposed `tab_ids` for targeted single-tab queries.
4. **`resources/read` Dense Curve Array Bloat (`crates/mcp/src/resources.rs`):**
   - Preserved default `result://<id>` payload compatibility (`fitted_curves[*].points` intact, pretty whitespace removed), and added URL query controls (`?include_curves=false`, `?detail=summary|compact|full`, `?tab_id=...`, `?max_rows=N`) on `sheet://` and `result://` URIs.
5. **Missing Parameter Discovery & Schema/Prompt Traps (`crates/engine/src/cmd/{mod.rs, analysis.rs}`, `crates/mcp/src/{prompts.rs, protocol.rs}`):**
   - Baseline `fit.run` JSON schema omitted the `method` parameter (`least_squares`, `robust`, `rout`) completely, and `stats.run` / `fit.run` `kind` & `equation_id` gave no shorthand/model examples. Added `method` to `fit.run`/`analysis.run` schemas and concrete examples to `stats.run`, `fit.run`, `open_file`, and `command_run`.
   - Baseline `prompts/get` failed with `-32602` when a client passed JSON booleans for `choose_two_group_test` (`paired`/`parametric`) because MCP prompt arguments must be strings; updated prompt descriptions and workflow steps in `prompts.rs` and `SERVER_INSTRUCTIONS` in `protocol.rs` (preserving strict `--gui-only` isolation).
6. **Silent Preamble/Text Hazards & Actionable Error Remediation (`crates/mcp/src/{formats.rs, tools.rs, protocol.rs}`, `crates/engine/src/cmd/{mod.rs, analysis.rs}`):**
   - Messy CSV imports with non-numeric text placeholders (`"ND"`) or preamble lines previously succeeded silently; `open_file` on numeric table archetypes now attaches `warnings` and a `next_step_hint` when non-numeric text cells appear.
   - Added shared `curvebook_engine::cmd::fuzzy_suggest` to emit `"Did you mean: ...?"` suggestions on unknown `stats.run`/`fit.run` `kind` (`anova_one_way` → `one_way_anova`) and unknown `command_run` IDs (`table.exclude` → `table.excludeCells`).
   - Added `command_list` schema-lookup hints on missing required fields (`table.excludeCellsWithReason` missing `reason_tag`) and disabled-toolset hints (`enable_toolset` / `command_run`) on `UnknownTool` JSON-RPC errors.

---

## 4. Deterministic Regression Coverage & Verification

- **Unit & Integration Tests Added:**
  - [`crates/mcp/tests/mcp_suite/r17_4_mcp_usability.rs`](../../crates/mcp/tests/mcp_suite/r17_4_mcp_usability.rs): covers `doc_inspect` (`detail`, `sections`, `tab_id`, `max_rows`, single-sheet Graph omission, compact `ResultCell` rendering, strict invalid-arg rejection), `command_list` / `list_toolsets` / `tools/list` schema compactness (`method` on `fit_run`, shorthands on `stats_run`), `resources/read` query bounds (`?include_curves=false`, `?max_rows=`, `?tab_id=`), messy-CSV `open_file` warnings, and fuzzy/schema-lookup error hints.
  - [`apps/curvebook-cli/tests/cli_suite/r17_4_stdio_workflows.rs`](../../apps/curvebook-cli/tests/cli_suite/r17_4_stdio_workflows.rs): drives a live `curvebook-cli mcp --workdir <dir>` stdio subprocess end-to-end across all four scientific workflows plus SVG/PDF/PNG signature verification.
- **Verification Status:** Verified locally via `cargo test -p curvebook-mcp`, targeted `curvebook-cli` MCP integration suites, `cargo test -p curvebook-cli --test cli_suite r17_4_stdio_workflows`, and `cargo xtask ci quick --base main` (and cleanroom/brands/telemetry/licenses checks).
