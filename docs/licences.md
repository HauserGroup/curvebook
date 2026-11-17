# Licence & Attribution Policy

CurveBook is dual-licensed under **`MIT OR Apache-2.0`** ([Cargo.toml](../Cargo.toml)). Every dependency, asset, sample dataset, and contribution must remain compatible with this licence without introducing copyleft restrictions.

---

## 1. Crate Dependency Allowlist

Only permissive, copy-left-free SPDX licences listed below are permitted for Rust crate dependencies across the workspace:

| SPDX Identifier | Scope | Notes |
| :--- | :--- | :--- |
| `MIT` | Code dependencies | Standard permissive licence |
| `Apache-2.0` | Code dependencies | Includes patent grant; check `NOTICE` propagation where present |
| `BSD-2-Clause` | Code dependencies | Permissive 2-clause BSD |
| `BSD-3-Clause` | Code dependencies | Permissive 3-clause BSD |
| `ISC` | Code dependencies | Permissive ISC |
| `Zlib` | Code dependencies | Permissive zlib compression/graphics crates |
| `Unicode-3.0` | Unicode / ICU tables | Approved Unicode Consortium data/code licence |
| `CC0-1.0` | Public-domain dedications | Public-domain equivalent crates/tables |

### Prohibited Licences
- **All copyleft licences are strictly banned** in runtime and build dependencies: `GPL-*`, `AGPL-*`, `LGPL-*` (regardless of version or "with classpath exception" variants), as well as clean-room ports/translations from copyleft projects ([AGENTS.md](../AGENTS.md)).
- Any SPDX identifier not listed in the allowlist above requires explicit maintainer audit and update to `deny.toml` and this file before `Cargo.lock` may introduce it.

---

## 2. Asset & Corpus Licences

Every non-code asset (icons, SVGs, cursor bitmaps, sample `.csv`/`.curvebook`/reference fixtures) must record its source and SPDX licence in [ASSETS.md](../ASSETS.md) (and [ATTRIBUTION.md](../ATTRIBUTION.md) for third-party works). Allowed asset/data licences include:
- `MIT`, `Apache-2.0`, `CC0-1.0`, `LicenseRef-Public-Domain` (e.g., NIST StRD statistical reference datasets)
- `CC-BY-4.0` and `CC-BY-SA-4.0` (where attribution is recorded in `ATTRIBUTION.md` and content remains separate from MIT/Apache-2.0 source code)
- `OFL-1.1`, `Ubuntu-font-1.0`, `Bitstream-Vera` (documentation/attribution references only; binary font files are **never** tracked in the git repository—see the zero-bundled-font rule in [AGENTS.md](../AGENTS.md))

---

## 3. Enforcement, Manifests, and Shipped Notice Files

| File / Tool | Role |
| :--- | :--- |
| `deny.toml` | Configuration for the full `cargo deny check` CI gate: licences, security advisories, yanked dependencies, bans, and sources. Optional dependencies are included through the all-features graph; advisory exceptions name their reason and a maintainer review date (`R0.5`, `R0.22`). |
| `about.toml` | `cargo-about` template configuration (paired with `about.hbs`) used to assemble bundled third-party crate licence disclosures (`R0.5`). |
| `cargo xtask licenses [--html [out]]` | Runs offline `deny.toml` / `Cargo.toml` / `Cargo.lock` licence checks, plus `cargo deny check licenses` when `cargo-deny` is installed (required when `CI=true`); `--html` writes `THIRD-PARTY-LICENSES.html` via `cargo about generate about.hbs` when `cargo-about` is installed, else a built-in pure-Rust renderer (`R0.5`). |
| `THIRD-PARTY-LICENSES.html` | Generated third-party Rust dependency licence compendium included in release packages and installers ([docs/release/releasing.md](release/releasing.md)). |
| [ATTRIBUTION.md](../ATTRIBUTION.md) | Third-party icon, palette, data, and scientific reference credits shipped alongside releases and checked against [ASSETS.md](../ASSETS.md). |
| [NOTICE](../NOTICE) | Apache-2.0 / ecosystem attribution notice included in source archives and release artefacts. |

Run `cargo xtask assets` on every PR that adds assets, and `cargo xtask licenses` before releasing or changing `Cargo.lock`. Also run `cargo deny check` when changing dependencies or the supply-chain policy; `xtask licenses` checks licensing and does not replace the advisory check. The full check updates advisory and registry data and requires network access; it runs in CI, outside the zero-egress application.
