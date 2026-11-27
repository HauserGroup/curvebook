# Privacy, security, licensing, and clean-room policy

<!-- Audience: Tier 2 Technical Reference — zero-egress architecture, local IPC trust boundaries, dependency licence rules, and vulnerability reporting belong here; keep ../index.html and ../faq.html concise for scientists. -->

CurveBook is built for confidential laboratory and clinical research data. The desktop app, the headless CLI, the MCP server, and the statistical libraries all run offline on your machine, with zero telemetry and zero network egress.

This reference covers local trust boundaries, build attestations, licensing, and vulnerability reporting. For a plain-language overview, see [CurveBook home](../index.html), the [FAQ](../faq.html), or the full [Privacy Policy](../privacy.html). The repository's own policies are in the [Security Policy](../../SECURITY.md) and the [Licence and Attribution Policy](../licences.md).

---

## Zero telemetry and zero network egress {#zero-egress}

CurveBook's binaries (`curvebook`, `curvebook-cli`) and core libraries contain no network clients, crash reporters, analytics, or background update checks.

- **Offline enforcement:** An automated repository check (`cargo xtask telemetry`) verifies that the source code and dependencies contain no telemetry, no HTTP client crates, and no outbound network calls.
- **Static documentation site:** The manual is static HTML and CSS, served with a strict Content-Security-Policy (`default-src 'none'`) and `Referrer-Policy: no-referrer`. It has no scripts, forms, cookies, or third-party web fonts.
- **Manual updates:** Shipped binaries never connect to the internet, so you install updates manually from verified release downloads ([Get started and installation](get-started.md#updating)).

---

## Local IPC, MCP confinement, and trust boundaries {#local-security}

### `--workdir` filesystem confinement (CLI and MCP) {#workdir-confinement}

- Passing `--workdir <dir>` to `curvebook-cli` or `curvebook-cli mcp` confines all reads, saves, archives, linked imports, and exports to the canonicalized `<dir>` folder.
- Path traversal (`..`), absolute paths outside `<dir>`, and symlinks that point outside `<dir>` fail with `path_escape`. CurveBook does not create missing parent directories in these cases.
- Without `--workdir`, commands run with the permissions of your OS user account. Always pass `--workdir <dir>` for AI assistants and untrusted MCP sessions (see [Automation: MCP](automation/mcp.md)).

### Loopback-only `--control` socket {#control-socket}

When you enable the control socket with `--control <port>` (or `CURVEBOOK_CONTROL_PORT`, with a port from `1` to `65535`):

- CurveBook binds only to `127.0.0.1:<port>` (loopback), never to `0.0.0.0` or an external interface.
- Every request needs a bearer token, which CurveBook checks in constant time. Set the token in `CURVEBOOK_CONTROL_TOKEN` rather than passing `--control-token` on the command line. Tokens given on the command line appear in process listings, and CurveBook warns when you use one.

---

## GitHub Pages and GitHub Releases policies {#pages-and-releases}

- **GitHub Pages:** The documentation is served as static pages directly from GitHub's infrastructure, with no application tracking, analytics beacons, or scripts. See the [Privacy Policy](../privacy.html) for details of the GitHub hosting.
- **GitHub Releases and build attestations:** GitHub Actions publishes the release packages, the CycloneDX 1.5 SBOM (`curvebook-<version>.cdx.json`), `THIRD-PARTY-LICENSES.html`, and `SHA256SUMS.txt`. Each release carries a keyless Sigstore build provenance attestation, and the platform packages carry CycloneDX SBOM attestations ([Releasing](../release/releasing.md)). The attestation steps run only in CI and add no network code to CurveBook binaries.

To verify the signed CycloneDX 1.5 SBOM attestation attached to an installer or a FreeBSD archive, run:

```sh
gh attestation verify <downloaded-file> \
  --repo HauserGroup/curvebook \
  --predicate-type https://cyclonedx.org/bom \
  --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
```

---

## Licensing, dependencies, and attributions {#licensing}

- **Dual licence:** CurveBook source code is dual-licensed under `MIT OR Apache-2.0` ([Cargo.toml](../../Cargo.toml), [NOTICE](../../NOTICE)).
- **Permissive dependency allowlist:** CI checks (`cargo deny check` and `cargo xtask licenses`) allow only dependencies with permissive SPDX licences (`MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`, `CC0-1.0`). Copyleft dependencies (`GPL-*`, `AGPL-*`, `LGPL-*`) are forbidden (see the [Licence and Attribution Policy](../licences.md)).
- **Release notices and fonts:** Releases include `THIRD-PARTY-LICENSES.html`, [`ATTRIBUTION.md`](../../ATTRIBUTION.md), and [`NOTICE`](../../NOTICE). CurveBook does not bundle any binary font files. Custom builds that use `CURVEBOOK_FONTS_DIR` read only fonts that the user supplies.

---

## Clean-room implementation statement {#clean-room}

CurveBook is an original, clean-room scientific software suite. Its statistical methods, nonlinear least-squares solvers, probability distributions, vector graphics renderers, container parsers, and UI components are written independently in Rust, based on peer-reviewed literature.

Citations appear throughout the numerical modules, the [Analysis Kinds Registry](registry/tables.html#analysis-kinds), and the [Parametric Equations Registry](registry/equations.html). Results are validated against NIST StRD and open-source reference oracles. Automated checks (`cargo xtask brands`, `cargo xtask cleanroom`, `cargo xtask citations`) enforce the clean-room boundaries continuously. Code that reads and writes `.pzfx files` and `.prism files` is kept at the file-format import and export boundary.

---

## Vulnerability reporting {#security-reporting}

Report security vulnerabilities, crashes, parser bugs, path-escape bypasses, or archive integrity defects as described in [SECURITY.md](../../SECURITY.md). The trust boundaries in scope include:

- Parsing of untrusted `.curvebook`, `.cbtemplate`, `.csv`, `.pzfx`, or `.prism` files (ZIP, XML, cell-count, and float checks)
- Path traversal or symlink escapes through `--workdir` or unpacked `.curvebook.d/` saves
- Loopback `--control` token checks and request framing caps
- Dimension and memory limits for PDF, SVG, PNG, TIFF, and EMF export
