//! Typed descriptor catalog for `curvebook-cli` subcommands, flags, and exit codes.
//!
//! Drives both `curvebook-cli --help` / per-subcommand `--help` output and the static
//! documentation site's CLI reference page so flag syntax, defaults, and exit codes stay in sync.

/// Canonical online documentation URL for `curvebook-cli`.
pub const CLI_DOCS_URL: &str = concat!(
    "https://hausergroup.github.io/curvebook/manual/v",
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
    "/automation/cli.html",
);

/// Canonical online registry URL containing per-subcommand `#cli-<id>` fragment targets.
pub const CLI_REGISTRY_DOCS_URL: &str = concat!(
    "https://hausergroup.github.io/curvebook/manual/v",
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
    "/registry/cli.html",
);

/// Descriptor for a CLI flag option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliFlagSpec {
    /// Long flag name including leading `--` (for example `"--workdir"`).
    pub long: &'static str,
    /// Optional short alias including leading `-` (for example `Some("-o")`).
    pub short: Option<&'static str>,
    /// Optional argument placeholder (for example `Some("<dir>")`), or `None` for boolean flags.
    pub value_placeholder: Option<&'static str>,
    /// Documented default behavior or value when omitted.
    pub default_value: &'static str,
    /// Concise behavior description.
    pub description: &'static str,
}

/// Descriptor for a CLI positional argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliPositionalSpec {
    /// Positional token shown in usage lines (for example `"<file>"`).
    pub syntax: &'static str,
    /// Whether the positional argument is mandatory.
    pub required: bool,
    /// Concise argument description.
    pub description: &'static str,
}

/// Descriptor for a `curvebook-cli` command form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliCommandSpec {
    /// Canonical command identifier (`"info"`, `"direct-command"`, `"mcp"`, etc.).
    pub id: &'static str,
    /// Usage synopsis line printed beneath `commands:` in `curvebook-cli --help`.
    pub usage_line: &'static str,
    /// Short one-line summary shown in `--help` and reference tables.
    pub summary: &'static str,
    /// Ordered positional arguments accepted by this subcommand.
    pub positionals: &'static [CliPositionalSpec],
    /// Ordered flags accepted by this subcommand.
    pub flags: &'static [CliFlagSpec],
}

/// Descriptor for a deterministic `curvebook-cli` process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliExitCodeSpec {
    /// Numeric POSIX exit status.
    pub code: u8,
    /// Symbolic constant name in `apps/curvebook-cli/src/error.rs`.
    pub name: &'static str,
    /// Conditions under which this exit status is returned.
    pub meaning: &'static str,
}

/// Top-level global options accepted by `curvebook-cli`.
pub const CLI_GLOBAL_FLAGS: &[CliFlagSpec] = &[
    CliFlagSpec { long: "--version", short: Some("-V"), value_placeholder: None, default_value: "—", description: "print version and exit" },
    CliFlagSpec { long: "--help", short: Some("-h"), value_placeholder: None, default_value: "—", description: "print this help message and exit" },
];

/// Deterministic exit codes returned by `curvebook-cli`.
pub const CLI_EXIT_CODES: &[CliExitCodeSpec] = &[
    CliExitCodeSpec { code: 0, name: "EXIT_OK", meaning: "Command completed cleanly." },
    CliExitCodeSpec {
        code: 1,
        name: "EXIT_RUNTIME",
        meaning: "Runtime, filesystem I/O, archive/format, solver, rendering, or engine execution error.",
    },
    CliExitCodeSpec {
        code: 2,
        name: "EXIT_USAGE_OR_POLICY",
        meaning: "Invalid CLI syntax, unknown flag/command, malformed batch/script schema, or `--workdir` path containment violation.",
    },
    CliExitCodeSpec { code: 130, name: "EXIT_INTERRUPTED", meaning: "Operation cancelled or interrupted (`SIGINT` / `ErrorCode::Cancelled`)." },
];

const FLAG_JSON: CliFlagSpec = CliFlagSpec {
    long: "--json",
    short: None,
    value_placeholder: None,
    default_value: "false",
    description: "Emit structured JSON output (and structured JSON errors on failure).",
};

const FLAG_WORKDIR: CliFlagSpec = CliFlagSpec {
    long: "--workdir",
    short: None,
    value_placeholder: Some("<dir>"),
    default_value: "unconstrained (current working directory)",
    description: "Confine all file reads, saves, exports, and external CSV/TSV links strictly within `<dir>`.",
};

const EXPORT_CONVERT_FLAGS: &[CliFlagSpec] = &[
    CliFlagSpec {
        long: "--out",
        short: Some("-o"),
        value_placeholder: Some("<out>"),
        default_value: "positional `<out>`",
        description: "Explicit output file or `.curvebook.d` directory path.",
    },
    CliFlagSpec {
        long: "--format",
        short: None,
        value_placeholder: Some("<fmt>"),
        default_value: "inferred from `<out>` extension",
        description: "Explicit target format (`curvebook`, `curvebook.d`, `svg`, `pdf`, `emf`, `png`, `tiff`, `csv`, `tsv`, `json`).",
    },
    CliFlagSpec {
        long: "--sheet",
        short: None,
        value_placeholder: Some("<id>"),
        default_value: "first matching sheet",
        description: "Target sheet ID or title to export.",
    },
    CliFlagSpec {
        long: "--xlsx-sheets",
        short: None,
        value_placeholder: Some("<names>"),
        default_value: "all visible sheets",
        description: "Comma-separated `.xlsx` worksheet names to import from an input workbook.",
    },
    CliFlagSpec {
        long: "--dpi",
        short: None,
        value_placeholder: Some("<u16>"),
        default_value: "300",
        description: "Rasterization resolution in dots per inch for `png` and `tiff` exports (18..=2400).",
    },
    CliFlagSpec {
        long: "--pdf-version",
        short: None,
        value_placeholder: Some("<1.4|1.7>"),
        default_value: "1.7",
        description: "PDF specification level for `pdf` exports (`1.4` or `1.7`).",
    },
    CliFlagSpec {
        long: "--ttf-font",
        short: None,
        value_placeholder: Some("<path>"),
        default_value: "none",
        description: "Optional TrueType `.ttf` font file embedded as `/FontFile2` in `pdf` exports.",
    },
    FLAG_JSON,
];

/// Complete typed catalog of `curvebook-cli` subcommands and flags.
pub const CLI_COMMANDS: &[CliCommandSpec] = &[
    CliCommandSpec {
        id: "info",
        usage_line: "info <file> [--list-sheets] [--json]",
        summary: "inspect tables, info sheets, analyses, graphs, layouts, or .xlsx workbook tabs",
        positionals: &[CliPositionalSpec {
            syntax: "<file>",
            required: true,
            description: "Input `.curvebook`, `.curvebook.d/`, `.cbtemplate`, `.prism`, `.pzfx`, `.xlsx`, `.csv`, `.tsv`, or project `.json` path.",
        }],
        flags: &[
            CliFlagSpec {
                long: "--list-sheets",
                short: None,
                value_placeholder: None,
                default_value: "false",
                description: "Enumerate worksheet tab names and indices in an `.xlsx` workbook.",
            },
            FLAG_JSON,
        ],
    },
    CliCommandSpec {
        id: "commands",
        usage_line: "commands [--group <group>] [--json]",
        summary: "list registered engine commands with schemas, effects and shortcuts",
        positionals: &[],
        flags: &[
            CliFlagSpec {
                long: "--group",
                short: None,
                value_placeholder: Some("<group>"),
                default_value: "all groups",
                description: "Filter by `CommandGroup` (`project`, `edit`, `sheet`, `table`, `info`, `analysis`, `graph`, `layout`, `export`).",
            },
            FLAG_JSON,
        ],
    },
    CliCommandSpec {
        id: "direct-command",
        usage_line: "<command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]",
        summary: "execute a registered engine command directly with schema-typed flags",
        positionals: &[CliPositionalSpec {
            syntax: "<command-id>",
            required: true,
            description: "Registered engine command identifier (for example `sheet.createData`, `stats.run`, `fit.run`).",
        }],
        flags: &[
            CliFlagSpec {
                long: "--in",
                short: None,
                value_placeholder: Some("<file>"),
                default_value: "fresh empty session",
                description: "Load an existing project or interchange file before executing the command.",
            },
            CliFlagSpec {
                long: "--out",
                short: None,
                value_placeholder: Some("<file>"),
                default_value: "none (in-memory only)",
                description: "Write the resulting project or sheet export to `<file>` after execution.",
            },
            FLAG_WORKDIR,
            FLAG_JSON,
            CliFlagSpec {
                long: "--<param>",
                short: None,
                value_placeholder: Some("<val>"),
                default_value: "per-command schema default",
                description: "Schema-typed command parameter flag (`--no-<param>` negates boolean parameters).",
            },
        ],
    },
    CliCommandSpec {
        id: "run",
        usage_line: "run <script> [--workdir <dir>] [--json]",
        summary: "execute a JSON-lines command stream headlessly",
        positionals: &[CliPositionalSpec {
            syntax: "<script>",
            required: true,
            description: "Path to an NDJSON or JSON-array command script file, or `-` to read from `stdin`.",
        }],
        flags: &[FLAG_WORKDIR, FLAG_JSON],
    },
    CliCommandSpec {
        id: "batch",
        usage_line: "batch <manifest> [--workdir <dir>] [--json]",
        summary: "execute a `curvebook.batch.v1` JSON manifest headlessly",
        positionals: &[CliPositionalSpec {
            syntax: "<manifest>",
            required: true,
            description: "Path to a `curvebook.batch.v1` JSON manifest file, or `-` to read from `stdin`.",
        }],
        flags: &[FLAG_WORKDIR, FLAG_JSON],
    },
    CliCommandSpec {
        id: "convert",
        usage_line: "convert <in> <out> [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]",
        summary: "convert archives/unpacked `.curvebook.d` dirs or export tables and figures (curvebook, curvebook.d, svg, pdf, emf, png, tiff, csv, tsv, json)",
        positionals: &[
            CliPositionalSpec { syntax: "<in>", required: true, description: "Input project archive, `.curvebook.d/`, template, or data file." },
            CliPositionalSpec { syntax: "<out>", required: false, description: "Output path (may instead be passed via `--out` / `-o`)." },
        ],
        flags: EXPORT_CONVERT_FLAGS,
    },
    CliCommandSpec {
        id: "export",
        usage_line: "export <in> (<out> | --out <out>) [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]",
        summary: "export a table, graph, layout, or project to svg, pdf, emf, png, tiff, csv, tsv, json, or .curvebook/.curvebook.d",
        positionals: &[
            CliPositionalSpec { syntax: "<in>", required: true, description: "Input project archive, `.curvebook.d/`, template, or data file." },
            CliPositionalSpec { syntax: "<out>", required: false, description: "Output path (or supply `--out <out>` / `-o <out>`)." },
        ],
        flags: EXPORT_CONVERT_FLAGS,
    },
    CliCommandSpec {
        id: "bench",
        usage_line: "bench <file> [--json]",
        summary: "measure parse, recalculation, compilation and export timings",
        positionals: &[CliPositionalSpec { syntax: "<file>", required: true, description: "Project file to benchmark." }],
        flags: &[FLAG_JSON],
    },
    CliCommandSpec {
        id: "perf",
        usage_line: "perf [--json] [--require-release]",
        summary: "run synthetic G12 performance checks",
        positionals: &[],
        flags: &[
            FLAG_JSON,
            CliFlagSpec {
                long: "--require-release",
                short: None,
                value_placeholder: None,
                default_value: "false",
                description: "Fail immediately unless `curvebook-cli` was compiled in `--release` mode.",
            },
        ],
    },
    CliCommandSpec {
        id: "mcp",
        usage_line: "mcp [--control-port <port>] [--workdir <dir>] [--control-token <token>]",
        summary: "run the Model Context Protocol (MCP) server over stdio (`--workdir` and `--control-port` are mutually exclusive)",
        positionals: &[],
        flags: &[
            CliFlagSpec {
                long: "--control-port",
                short: None,
                value_placeholder: Some("<port>"),
                default_value: "CURVEBOOK_CONTROL_PORT env or headless",
                description: "Attach over loopback `127.0.0.1:<port>` to a running `curvebook --control <port>` window (`1..=65535`; mutually exclusive with `--workdir`).",
            },
            CliFlagSpec {
                long: "--workdir",
                short: None,
                value_placeholder: Some("<dir>"),
                default_value: "unconfined",
                description: "Confine all MCP file/command paths strictly within directory `<dir>` (mutually exclusive with `--control-port`, which attaches to an external desktop host).",
            },
            CliFlagSpec {
                long: "--control-token",
                short: None,
                value_placeholder: Some("<token>"),
                default_value: "CURVEBOOK_CONTROL_TOKEN env or control-<port>.token",
                description: "Bearer token used to authenticate against a `--control` desktop listener.",
            },
        ],
    },
];

/// Look up a CLI subcommand descriptor by `id`.
#[must_use]
pub fn find_cli_command(id: &str) -> Option<&'static CliCommandSpec> {
    CLI_COMMANDS.iter().find(|cmd| cmd.id == id)
}

/// Format top-level `curvebook-cli --help` text directly from the typed descriptor table.
#[must_use]
pub fn format_usage() -> String {
    use std::fmt::Write as _;

    let mut out = String::from("usage: curvebook-cli <command> [args]\n\ncommands:\n");
    for cmd in CLI_COMMANDS {
        let _ = writeln!(out, "  {}", cmd.usage_line);
        let _ = writeln!(out, "                    {}", cmd.summary);
    }
    out.push_str("\noptions:\n");
    for opt in CLI_GLOBAL_FLAGS {
        let label = match opt.short {
            Some(short) => format!("{short}, {}", opt.long),
            None => opt.long.to_owned(),
        };
        let _ = writeln!(out, "  {label:<18}{}", opt.description);
    }
    out.push_str("\nexit codes:\n");
    for ec in CLI_EXIT_CODES {
        let tag = format!("{} ({})", ec.code, ec.name);
        let _ = writeln!(out, "  {tag:<28}{}", ec.meaning);
    }
    let _ = writeln!(out, "\ndocs: {CLI_DOCS_URL}");
    out
}

/// Format detailed help text for a specific subcommand descriptor.
#[must_use]
pub fn format_subcommand_help(id: &str) -> Option<String> {
    use std::fmt::Write as _;

    let cmd = find_cli_command(id)?;
    let mut out = format!("usage: curvebook-cli {}\n\n{}\n", cmd.usage_line, cmd.summary);
    if !cmd.positionals.is_empty() {
        out.push_str("\narguments:\n");
        for pos in cmd.positionals {
            let req = if pos.required { "required" } else { "optional" };
            let _ = writeln!(out, "  {:<18} ({req}) {}", pos.syntax, pos.description);
        }
    }
    if !cmd.flags.is_empty() {
        out.push_str("\nflags:\n");
        for flag in cmd.flags {
            let mut label = match flag.short {
                Some(short) => format!("{short}, {}", flag.long),
                None => flag.long.to_owned(),
            };
            if let Some(ph) = flag.value_placeholder {
                label.push(' ');
                label.push_str(ph);
            }
            let _ = writeln!(out, "  {label:<26} {} [default: {}]", flag.description, flag.default_value);
        }
    }
    let _ = writeln!(out, "\ndocs: {CLI_REGISTRY_DOCS_URL}#cli-{}", cmd.id);
    Some(out)
}
