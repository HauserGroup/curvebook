# Privacy, Security, Licensing & Clean-Room Policy

CurveBook is engineered for confidential laboratory and clinical research data: the desktop application, headless CLI, MCP server, and statistical libraries run offline on your machine with zero telemetry and zero network egress.

See the public [Privacy Policy](https://hausergroup.github.io/curvebook/privacy.html), [Security Policy](../../SECURITY.md), and [Licence & Attribution Policy](../licences.md).

---

## Zero Telemetry & Zero Network Egress {#zero-egress}

CurveBook's application binaries (`curvebook`, `curvebook-cli`) and core libraries contain **no** network clients, **no** crash reporters, **no** usage analytics, and **no** background update checks:

- **Offline enforcement:** Enforced in CI via `cargo xtask telemetry`, checking source code and dependency trees for telemetry, HTTP client crates, or remote calls.
- **No external telemetry in documentation:** The documentation site uses pure static HTML/CSS with strict Content-Security-Policy (`default-src 'none'`), `Referrer-Policy: no-referrer`, zero `<script>` tags, zero `<form>` elements, zero cookies, and zero third-party web fonts.
- **Updates:** Because shipped binaries make zero outbound HTTP requests, updates are installed manually from verified release downloads ([Get Started & Installation](get-started.md#updating)).

---

## Local IPC, MCP Confinement & Trust Boundaries {#local-security}

### `--workdir` Filesystem Confinement (CLI & MCP) {#workdir-confinement}

- Launching `curvebook-cli mcp --workdir <dir>` or `curvebook-cli ... --workdir <dir>` confines every file read, write, archive open, external import link, and figure export strictly inside the canonicalized `<dir>` boundary (`crates/engine/src/path_policy.rs`, `crates/mcp/src/formats.rs`).
- Relative escape segments (`..`), absolute paths outside `<dir>`, and symlinks resolving outside `<dir>` fail with `ErrorCode::PathEscape` (`path_escape`) without creating missing parent directories.
- Without `--workdir`, commands execute with your OS user account permissions (`Unconstrained local-user mode`); AI assistants and untrusted MCP prompts should always run with `--workdir <dir>` (see [Automation: MCP](automation/mcp.md)).

### Loopback-Only `--control` Socket {#control-socket}

When enabled via `--control <port>` (or `CURVEBOOK_CONTROL_PORT`, `port` in `1..=65535`):

- Binds strictly to **`127.0.0.1:<port>`** (loopback interface only — never `0.0.0.0` or external NICs).
- Authenticates JSON-lines commands with a constant-time bearer token (`CURVEBOOK_CONTROL_TOKEN` environment variable recommended; passing `--control-token` on the CLI exposes the secret in `ps` process listings and triggers a stderr warning).
- Combining `--gui-only` blocks direct non-GUI engine mutations (`ErrorCode::GuiOnlyViolation`) so external automation drives only interactive GUI actions.

---

## GitHub Pages & GitHub Releases Policies {#pages-and-releases}

- **GitHub Pages:** Serves static documentation (`docs/index.html`, `docs/privacy.html`, `docs/manual/`) directly via GitHub infrastructure with zero application tracking, zero analytics beacons, and strict zero-script rendering (`privacy.html` documents GitHub hosting metadata policies).
- **GitHub Releases & Build Attestations:** Every Beta release package and `SHA256SUMS.txt` is published through GitHub Actions (`release-scan` security gate) and receives a keyless OIDC/Sigstore build provenance attestation (`actions/attest-build-provenance`, documented in [Releasing](../release/releasing.md)). Attestation calls run strictly inside CI workflows (`id-token: write`, `attestations: write`), adding zero network behavior to compiled CurveBook binaries.

---

## Licensing, Dependencies & Attributions {#licensing}

- **Dual Licence:** CurveBook source code is dual-licensed under **`MIT OR Apache-2.0`** ([Cargo.toml](../../Cargo.toml), [NOTICE](../../NOTICE)).
- **Permissive Dependency Allowlist:** Checked continuously via `cargo deny check` (`deny.toml`) and `cargo xtask licenses`, allowing only permissive SPDX licences (`MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`, `CC0-1.0`) and prohibiting all copyleft dependencies (`GPL-*`, `AGPL-*`, `LGPL-*`). See [Licence & Attribution Policy](../licences.md).
- **Release Notices:** Every release includes `THIRD-PARTY-LICENSES.html` alongside [`ATTRIBUTION.md`](../../ATTRIBUTION.md) and [`NOTICE`](../../NOTICE). CurveBook repositories and binaries contain **zero bundled binary font files** (`cargo xtask assets`); custom builds using `CURVEBOOK_FONTS_DIR` embed only explicitly supplied `OFL-1.1` font files.

---

## Clean-Room Implementation Statement {#clean-room}

CurveBook is an original clean-room scientific software suite. Statistical methods, nonlinear least-squares solvers, distribution evaluators, vector graphics renderers, file container parsers, and user interface components are implemented independently in Rust from published peer-reviewed academic literature (`D1` citations documented in the numerical modules, [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and [Parametric Equations Registry](registry/equations.html)), validated against public NIST StRD and open-source reference oracles, and verified continuously via `cargo xtask brands`, `cargo xtask cleanroom`, and `cargo xtask citations`. Support for `.pzfx` and `.prism` files is implemented purely at the external file-format import boundary (`crates/format`).

---

## Vulnerability Reporting {#security-reporting}

Security vulnerabilities, crash bugs, parser exploits, path-escape bypasses, or archive integrity defects should be reported following [SECURITY.md](../../SECURITY.md). In-scope threat boundaries include:

- Untrusted `.curvebook`, `.cbtemplate`, `.csv`, `.pzfx`, or `.prism` archive/file parsing (bounded ZIP, XML, cell-count, and float-finite checks)
- Path traversal or symlink escape across `--workdir` or unpacked `.curvebook.d/` saves
- Loopback `--control` token validation and `--gui-only` enforcement
- PDF / SVG / PNG / TIFF / EMF export integer/pixel/memory resource limits.
