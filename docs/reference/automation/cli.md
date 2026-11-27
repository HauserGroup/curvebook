# CLI reference (`curvebook-cli`)

<!-- Audience: Tier 2 Technical Reference — complete CLI subcommands, schema-typed flags, exit codes, and export options belong here; keep ../get-started.md and ../../linux.html concise for scientists. -->

`curvebook-cli` is CurveBook's command-line runner and stdio MCP server. It works without opening the desktop window. Use it to inspect projects, run analyses, export figures, or process many files in a batch.

If you have not installed `curvebook-cli` yet, see [Install CurveBook](../get-started.md). The [CLI Reference Registry](../registry/cli.html) and `curvebook-cli --help` both draw their command names, usage lines, flags, defaults, and exit codes from the same typed catalog.

## Global syntax and options

The top-level invocation forms are:

```sh
curvebook-cli <command> [args]
curvebook-cli <command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]
curvebook-cli --help
curvebook-cli --version
```

- `-h`, `--help`, `help [subcommand]`: Prints the command catalog, or the help for one subcommand, and exits with `0`.
- `-V`, `--version`, `version`: Prints the `curvebook-cli` version and exits with `0`.

## Subcommands summary

Every subcommand accepts `-h` or `--help`. Each one runs as a single call that gives the same result every time for the same input:

- `info <file> [--json]` — Lists the tables, info sheets, analyses, graphs, and layouts in an archive or project file.
- `commands [--group <group>] [--json]` — Lists the registered engine commands with their schemas, effects, and shortcuts (see the [Engine Command Registry](../registry/commands.html)).
- `<command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]` — Runs any engine command directly. Its parameters become flags typed by the command's schema. Use `--no-<param>` to set a boolean flag to `false`.
- `run <script> [--workdir <dir>] [--json]` — Runs a stream of commands, written as NDJSON or as a JSON array, without the desktop window. Use `-` as the script name to read from standard input.
- `batch <manifest> [--workdir <dir>] [--json]` — Runs a `curvebook.batch.v1` manifest without the desktop window (see the [control protocol and batch reference](control-and-batch.md#batch-manifest)).
- `convert <in> <out> [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]` — Converts archives, `.curvebook.d/` folders, templates, or data files. It can also export tables and figures.
- `export <in> (<out> | --out <out>) [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]` — Exports a table, graph, layout, or project. Supported targets include SVG, PDF, EMF, PNG, TIFF, CSV, TSV, JSON, `.curvebook`, and `.curvebook.d`.
- `bench <file> [--json]` — Times parsing, recalculation, scene compilation, and export for a project file.
- `perf [--json] [--require-release]` — Runs synthetic checks against the performance budgets.
- `mcp [--control-port <port>] [--workdir <dir>] [--control-token <token>]` — Starts the stdio [MCP server](mcp.md).

The [CLI Reference Registry](../registry/cli.html) lists the full flags, defaults, and option descriptions for every subcommand.

## Direct command flag mapping

You can call any engine command directly, for example `curvebook-cli sheet.createData --in proj.curvebook --out proj.curvebook --kind xy`. CurveBook checks each flag against that command's JSON Schema:

- String, number, and array options accept `--<param> <value>` or `--<param>=<value>`. Structured fields accept JSON arrays or objects.
- Boolean options accept `--<param>` (`true`), `--<param>=true|false`, or `--no-<param>` (`false`).
- Unknown options, type mismatches, and paths that escape `--workdir` fail before anything is changed, and the command exits with code 2.

## Exit codes and structured JSON errors {#exit-codes}

| Code | Name | Condition |
|---:|---|---|
| `0` | Success | The command finished successfully. |
| `1` | Runtime error | I/O failure, invalid file format, solver failure, or engine error. |
| `2` | Usage or policy error | Invalid flag, unknown command, bad batch schema, or `--workdir` escape. |
| `130` | Interrupted | Cancelled by `SIGINT` or by a cancellation request. |

By default, errors appear as plain text on standard error. Pass `--json` before `--` to get structured JSON errors on standard output instead. Each error object has the shape `{"error":{"code","category","message","retryable","remediation","details"}}`.

## Format and PDF options

Supported file formats include `.curvebook`, `.curvebook.d`, `.cbtemplate`, `.pzfx`, `.prism`, `.csv`, `.tsv`, `.svg`, `.pdf`, `.emf`, `.png`, `.tiff`, and `.json`. `.prism files` can be read but not written at Beta. For full details, see the [Formats & Schemas Registry](../registry/formats.html) and [Files, import, and export](../files.md). PDF export uses version `1.7` by default; choose a version with `--pdf-version 1.4|1.7`. To embed a subset of a TrueType font in the PDF, pass `--ttf-font <path>`.
