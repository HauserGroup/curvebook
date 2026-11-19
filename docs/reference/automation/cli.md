# CLI Reference (`curvebook-cli`)

`curvebook-cli` is the headless command-line runner and stdio MCP entry point for CurveBook. Command names, usage lines, flags, defaults, and exit codes in the [CLI Reference Registry](../registry/cli.html) and in `curvebook-cli --help` are generated from the same typed Rust catalog (`cli_catalog.rs`).

## Global Syntax and Options

Single-call top-level invocation forms:

```sh
curvebook-cli <command> [args]
curvebook-cli <command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]
curvebook-cli --help
curvebook-cli --version
```

- `-h`, `--help`, `help [subcommand]`: Print the command catalog or subcommand flag help and exit with `0`.
- `-V`, `--version`, `version`: Print the `curvebook-cli` package version and exit with `0`.

## Subcommands Summary

Every built-in subcommand accepts `-h` / `--help` and follows deterministic single-call flag syntax:

- `info <file> [--json]` — Inspect tables, info sheets, analyses, graphs, and layouts in an archive or project file.
- `commands [--group <group>] [--json]` — List registered engine commands with schemas, effects, and shortcuts (see the [Engine Command Registry](../registry/commands.html)).
- `<command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]` — Invoke any registered engine command directly with schema-typed flags (`--no-<param>` negates booleans).
- `run <script> [--workdir <dir>] [--json]` — Execute an NDJSON or JSON-array command stream headlessly (`-` reads `stdin`).
- `batch <manifest> [--workdir <dir>] [--json]` — Execute a `curvebook.batch.v1` manifest headlessly (see [Control Protocol & Batch Reference](control-and-batch.md#batch-manifest)).
- `convert <in> <out> [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]` — Convert archives, `.curvebook.d/` unpacked directories, templates, or interchange data files, or export tables and figures.
- `export <in> (<out> | --out <out>) [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]` — Export a specific table, graph, layout, or project to SVG, PDF, EMF, PNG, TIFF, CSV, TSV, JSON, `.curvebook`, or `.curvebook.d`.
- `bench <file> [--json]` — Measure parse, recalculation, scene compilation, and export timings on a project file.
- `perf [--json] [--require-release]` — Run synthetic performance budget checks.
- `mcp [--control-port <port>] [--gui-only] [--workdir <dir>] [--control-token <token>]` — Start the stdio [MCP Server](mcp.md).

Full argument, flag, default value, and option descriptions for every subcommand live in the generated [CLI Reference Registry](../registry/cli.html).

## Direct Command Flag Mapping

When invoking an engine command ID directly on the command line (for example `curvebook-cli sheet.createData --in proj.curvebook --out proj.curvebook --kind xy`), CLI flags are validated against the command's JSON parameter schema:

- String, integer, float, and array parameters accept `--<param> <value>` or `--<param>=<value>` (JSON arrays/objects are parsed when the schema expects structured fields).
- Boolean parameters accept `--<param>` (sets `true`), `--<param>=true|false`, or `--no-<param>` (sets `false`).
- Unknown parameters, wrong types, or `--workdir` path escapes fail immediately before mutation with exit code `2`.

## Exit Codes and Structured JSON Errors {#exit-codes}

| Code | Constant | Condition |
|---|---|---|
| `0` | `EXIT_OK` | Command completed cleanly. |
| `1` | `EXIT_RUNTIME` | I/O error, archive/format error, solver/math failure, or engine runtime error. |
| `2` | `EXIT_USAGE_OR_POLICY` | Invalid CLI syntax, unknown flag/command, malformed batch schema, or `--workdir` containment violation. |
| `130` | `EXIT_INTERRUPTED` | Cancelled or interrupted (`SIGINT` / `ErrorCode::Cancelled`). |

Passing `--json` before `--` guarantees that failures write a structured JSON `CliErrorEnvelope` (`{"error":{"code","category","message","retryable","remediation","details"}}`) to standard output rather than plain text on standard error.

## Format and PDF Options

For interchange format descriptors and accepted file extensions (`.curvebook`, `.curvebook.d`, `.cbtemplate`, `.pzfx`, `.prism`, `.csv`, `.tsv`, `.svg`, `.pdf`, `.emf`, `.png`, `.tiff`, `.json`), see the [Formats & Schemas Registry](../registry/formats.html) and [Files, Import & Export](../files.md). PDF export defaults to version `1.7` (`--pdf-version 1.4|1.7`) and accepts `--ttf-font <path>` for TrueType subset embedding.
