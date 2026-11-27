# Privacy, security, licensing, and clean-room policy

<!-- Audience: Tier 2 Technical Reference — zero-egress architecture, local IPC trust boundaries, dependency licence rules, and vulnerability reporting belong here; keep ../index.html and ../faq.html concise for scientists. -->

CurveBook is built for confidential laboratory and clinical research data. The desktop app, headless CLI, MCP server, and statistical libraries run offline on your machine with zero telemetry and zero network egress.

This reference covers local trust boundaries, build attestations, licensing, and vulnerability reporting. For a plain-language overview, see [CurveBook home](../index.html), the [FAQ](../faq.html), or the full [Privacy Policy](../privacy.html). Repository policies also live in [Security Policy](../../SECURITY.md) and [Licence & Attribution Policy](../licences.md).

---

## Zero telemetry and zero network egress {#zero-egress}

CurveBook's binaries (`curvebook`, `curvebook-cli`) and core libraries contain **no** network clients, crash reporters, analytics, or background update checks:

- **Offline enforcement:** Automated repository checks (`cargo xtask telemetry`) verify that source code and dependencies contain no telemetry, HTTP client crates, or outbound network calls.
- **Static documentation site:** The manual uses static HTML and CSS with a strict Content-Security-Policy (`default-src 'none'`), `Referrer-Policy: no-referrer`, zero scripts, zero forms, zero cookies, and zero third-party web fonts.
- **Manual updates:** Because shipped binaries never connect to the internet, install updates manually from verified release downloads ([Get started and installation](get-started.md#updating)).

---

## Local IPC, MCP confinement, and trust boundaries {#local-security}

### `--workdir` filesystem confinement (CLI and MCP) {#workdir-confinement}

- Passing `--workdir <dir>` to `curvebook-cli` or `curvebook-cli mcp` confines all reads, saves, archives, linked imports, and exports inside the canonicalized `<dir>` folder.
- Path traversal (`..`), external absolute paths, and symlinks pointing outside `<dir>` fail with `path_escape` without creating missing parent directories.
- Without `--workdir`, commands run with your OS user account permissions. Always pass `--workdir <dir>` for AI assistants and untrusted MCP sessions (see [Automation: MCP](automation/mcp.md)).

### Loopback-only `--control` socket {#control-socket}

When enabled with `--control <port>` (or `CURVEBOOK_CONTROL_PORT`, port `1` to `65535`):

- CurveBook binds strictly to **`127.0.0.1:<port>`** (loopback only, never `0.0.0.0` or external interfaces).
- Requests require a bearer token checked in constant time. Set `CURVEBOOK_CONTROL_TOKEN` rather than passing `--control-token` on the command line. Command-line tokens appear in process listings and trigger a warning.

---

## GitHub Pages and GitHub Releases policies {#pages-and-releases}

- **GitHub Pages:** Serves static documentation pages directly from GitHub infrastructure with zero application tracking, analytics beacons, or scripts. See the [Privacy Policy](../privacy.html) for GitHub hosting details.
- **GitHub Releases and build attestations:** Release packages, the CycloneDX 1.5 SBOM (`curvebook-<version>.cdx.json`), `THIRD-PARTY-LICENSES.html`, and `SHA256SUMS.txt` are published through GitHub Actions. Each release carries a keyless Sigstore build provenance attestation, and platform packages carry CycloneDX SBOM attestations ([Releasing](../release/releasing.md)). Attestation steps run only in CI and add zero network code to CurveBook binaries.

To verify the signed CycloneDX 1.5 SBOM attestation attached to an installer or FreeBSD archive:

```sh
gh attestation verify <downloaded-file> \
  --repo HauserGroup/curvebook \
  --predicate-type https://cyclonedx.org/bom \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

---

## Licensing, dependencies, and attributions {#licensing}

- **Dual licence:** CurveBook source code is dual-licensed under **`MIT OR Apache-2.0`** ([Cargo.toml](../../Cargo.toml), [NOTICE](../../NOTICE)).
- **Permissive dependency allowlist:** CI checks (`cargo deny check` and `cargo xtask licenses`) enforce permissive SPDX licences only (`MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`, `CC0-1.0`). Copyleft dependencies (`GPL-*`, `AGPL-*`, `LGPL-*`) are forbidden (see [Licence & Attribution Policy](../licences.md)).
- **Release notices and fonts:** Releases include `THIRD-PARTY-LICENSES.html`, [`ATTRIBUTION.md`](../../ATTRIBUTION.md), and [`NOTICE`](../../NOTICE). CurveBook ships **zero bundled binary font files**; custom builds using `CURVEBOOK_FONTS_DIR` read only user-supplied fonts.

---

## Clean-room implementation statement {#clean-room}

CurveBook is an original clean-room scientific software suite. Statistical methods, nonlinear least-squares solvers, probability distributions, vector graphics renderers, container parsers, and UI components are written independently in Rust from peer-reviewed literature.

Citations appear across the numerical modules, the [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and the [Parametric Equations Registry](registry/equations.html). Results are validated against NIST StRD and open-source reference oracles. Automated checks (`cargo xtask brands`, `cargo xtask cleanroom`, `cargo xtask citations`) enforce clean-room boundaries continuously. Support for `.pzfx files` and `.prism files` is isolated at the external file-format import boundary.

---

## Vulnerability reporting {#security-reporting}

Report security vulnerabilities, crashes, parser bugs, path-escape bypasses, or archive integrity defects according to [SECURITY.md](../../SECURITY.md). In-scope trust boundaries include:

- Parsing untrusted `.curvebook`, `.cbtemplate`, `.csv`, `.pzfx`, or `.prism` files (ZIP, XML, cell-count, and float checks)
- Path traversal or symlink escapes across `--workdir` or unpacked `.curvebook.d/` saves
- Loopback `--control` token checks and request framing caps
- PDF, SVG, PNG, TIFF, and EMF export dimension and memory limits.
