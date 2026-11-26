# CLI reference (`curvebook-cli`)

`curvebook-cli` is CurveBook's headless command-line runner and stdio MCP server. Command names, usage lines, flags, defaults, and exit codes in the [CLI Reference Registry](../registry/cli.html) and `curvebook-cli --help` come from the same typed catalog.

## Global syntax and options

Top-level invocation forms:

```sh
curvebook-cli <command> [args]
curvebook-cli <command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]
curvebook-cli --help
curvebook-cli --version
```

- `-h`, `--help`, `help [subcommand]`: Print the command catalog or subcommand help and exit with `0`.
- `-V`, `--version`, `version`: Print the `curvebook-cli` version and exit with `0`.

## Subcommands summary

Every subcommand accepts `-h` or `--help` and runs in a single deterministic call:

- `info <file> [--json]` — Inspect tables, info sheets, analyses, graphs, and layouts in an archive or project file.
- `commands [--group <group>] [--json]` — List registered engine commands with schemas, effects, and shortcuts (see the [Engine Command Registry](../registry/commands.html)).
- `<command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]` — Run any engine command directly with schema-typed flags (`--no-<param>` sets booleans to `false`).
- `run <script> [--workdir <dir>] [--json]` — Run an NDJSON or JSON-array command stream headlessly (`-` reads standard input).
- `batch <manifest> [--workdir <dir>] [--json]` — Run a `curvebook.batch.v1` manifest headlessly (see [Control protocol & batch reference](control-and-batch.md#batch-manifest)).
- `convert <in> <out> [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]` — Convert archives, `.curvebook.d/` folders, templates, or data files, or export tables and figures.
- `export <in> (<out> | --out <out>) [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]` — Export a table, graph, layout, or project to SVG, PDF, EMF, PNG, TIFF, CSV, TSV, JSON, `.curvebook`, or `.curvebook.d`.
- `bench <file> [--json]` — Time parsing, recalculation, scene compilation, and export on a project file.
- `perf [--json] [--require-release]` — Run synthetic performance budget checks.
- `mcp [--control-port <port>] [--workdir <dir>] [--control-token <token>]` — Start the stdio [MCP server](mcp.md).

Full flags, defaults, and option descriptions for every subcommand appear in the [CLI Reference Registry](../registry/cli.html).

## Direct command flag mapping

When calling an engine command directly (for example `curvebook-cli sheet.createData --in proj.curvebook --out proj.curvebook --kind xy`), flags are validated against its JSON schema:

- String, number, and array options accept `--<param> <value>` or `--<param>=<value>`. Structured fields accept JSON arrays or objects.
- Boolean options accept `--<param>` (`true`), `--<param>=true|false`, or `--no-<param>` (`false`).
- Unknown options, type mismatches, and `--workdir` path escapes fail before mutating anything and exit with code 2.

## Exit codes and structured JSON errors {#exit-codes}

| Code | Name | Condition |
|---:|---|---|
| `0` | Success | Command finished cleanly. |
| `1` | Runtime error | I/O failure, invalid file format, solver failure, or engine error. |
| `2` | Usage or policy error | Invalid flag, unknown command, bad batch schema, or `--workdir` escape. |
| `130` | Interrupted | Cancelled by `SIGINT` or cancellation request. |

Pass `--json` before `--` to emit failures as a structured JSON error envelope (`{"error":{"code","category","message","retryable","remediation","details"}}`) on standard output instead of plain text on standard error.

## Format and PDF options

For supported file formats and extensions (`.curvebook`, `.curvebook.d`, `.cbtemplate`, `.pzfx`, `.prism`, `.csv`, `.tsv`, `.svg`, `.pdf`, `.emf`, `.png`, `.tiff`, `.json`), see the [Formats & Schemas Registry](../registry/formats.html) and [Files, import & export](../files.md). PDF export defaults to version `1.7` (`--pdf-version 1.4|1.7`) and accepts `--ttf-font <path>` to embed a TrueType font subset.
