# Clean-Room, Branding & Reference Policy

This document holds the full clean-room, naming, upstream source-reuse, and reference-material rules summarized in [`AGENTS.md`](../AGENTS.md) §1. Enforced by `cargo xtask cleanroom`, `cargo xtask brands`, and `cargo xtask assets`.

---

## Independent Implementation & Copyleft Prohibition

1. **Independent implementation and original design.** Never copy, translate, decompile, disassemble, or inspect the binaries, memory, or bundled assets of proprietary commercial statistical, graphing, or curve-fitting applications. Permitted design sources are publicly observable file format behaviour (such as `.pzfx` and `.prism` files read/written by `crates/format`), public scientific specifications, peer-reviewed statistical literature, and the narrowly limited local JPG references defined below. Keep CurveBook's visual design original. The [Attributed VectorCraft GUI source reuse](#attributed-vectorcraft-gui-source-reuse) policy below governs the sole exception to independent implementation.
2. **No copyleft dependencies or translations.** Never depend on or translate code from `GPL`, `AGPL`, or `LGPL` projects—even permissive-looking ports of GPL codebases (e.g., R, GNU Scientific Library, JASP, PSPP, Veusz, LabPlot, SciDAVis).

---

## Neutral Terminology, Brand Identity & `.pzfx` / `.prism` Naming

Write code identifiers, UI labels, comments, examples, and documentation in CurveBook's own words. Exclude other suite brands, placeholder template package names, and proprietary competitor product names from the repository except for the source notices and policy identifiers explicitly permitted below (`cargo xtask brands` and `cargo xtask cleanroom` enforce this).

- **`.prism files` and `.pzfx files` are specifically allowed over clean-room and branding principles:** the lowercase file extensions `.prism` and `.pzfx` (for example `.prism files`, `.pzfx files`, `.prism`, `*.prism`, `experiment.prism`) are explicitly permitted on every surface—including UI menus, dialogs, tooltips, documentation, tutorials, and page titles/descriptions—and **all user-facing documentation and GUI copy should prefer `".pzfx files"` and `".prism files"` naming where appropriate** rather than opaque euphemisms such as `"External Archive"` or `"XML Project"`. Do **not** avoid or rewrite `.prism files` out of clean-room or branding caution.
- **Boundaries that still apply:** bare or capitalised competitor product names, capitalised extension spellings, vendor documentation prose or screenshots, and vendor-generated `.prism`/`.pzfx`/`.pzf` sample files remain strictly forbidden (`cargo xtask brands` and `cargo xtask cleanroom`); command and XML identifiers using that token stay strictly limited to file format parser/exporter boundaries. The local archive identifier in [Limited local JPG reference policy](#limited-local-jpg-reference-policy) is permitted solely to define that policy's scope.
- **Single bare `.pzfx` XML root-element constant (decided 2026-11-26):** solely for `.pzfx` export interoperability, the single bare vendor XML root-element name constant (never `<`-prefixed) is permitted exclusively in `crates/format/src/pzfx/export.rs` and referenced by the `.pzfx` importer and tests without re-spelling the literal. All other tracked source files, docs, fixtures, packaged text/archives, copyright or creator notices, and `<`-prefixed root-element XML tags remain strictly forbidden everywhere (`cargo xtask brands` and `cargo xtask cleanroom` enforce both boundaries).
- **Homepage migration heading & trademark disclaimer:** solely in visible body text of `docs/index.html` (never in `<title>`, `<meta>`, HTML comments, attributes, or any other tracked file), `xtask/src/brands.rs` permits the exact migration-section heading and matching footer non-affiliation disclaimer.

---

## Attributed VectorCraft GUI Source Reuse

- Contributors may copy or adapt permissively licensed (`MIT OR Apache-2.0`) VectorCraft GUI helpers, widget geometry, layout proportions, spacing/typography tokens, and tests into CurveBook. Verify the licence of the actual source files and preserve required copyright, licence, and upstream notices. Record the exact upstream VectorCraft repository URL, commit revision, original file paths, and affected CurveBook files in [`ATTRIBUTION.md`](../ATTRIBUTION.md), and retain required licence text in the shipped [`NOTICE`](../NOTICE).
- The exact upstream VectorCraft GitHub repository URL is permitted solely inside [`ATTRIBUTION.md`](../ATTRIBUTION.md) and [`NOTICE`](../NOTICE) (enforced by `cargo xtask brands` and `cargo xtask cleanroom`); never place external suite org/crate names or doc-comment references in source code, UI strings, documentation, `AGENTS.md`, or this file. Keep CurveBook's own UI terminology, product identity, scientific colour palette, icons, and assets. This permission covers source code and tests only; proprietary-source restrictions, workspace-wide `unsafe_code = "forbid"`, registry-only dependency allowlist ([`deny.toml`](../deny.toml)), zero telemetry, and asset/zero-bundled-font policies remain in force.

---

## Limited local JPG reference policy

- **Scope:** Existing, lawfully obtained JPG/JPEG reference frames beneath the gitignored local `plan/prism/` directory may be inspected solely to understand high-level workflows, information hierarchy, and generic functional organization. This permission does not override access terms, licences, confidentiality obligations, or third-party rights, and does not extend to other archive contents or proprietary installations.
- **Independent creation:** Implement every feature independently and create original screen compositions, wording, icons, diagrams, and styling. Do not trace, extract, reuse, reproduce, or closely imitate distinctive screen arrangements, proportions, visual treatments, branding, tutorial examples, or other expressive elements from the frames. Generic functional organization is the limit of the layout reference permission.
- **Local handling and provenance:** Keep the frames and their provenance notes gitignored and out of source commits, fixtures, generated assets, packages, and distribution. Record source URLs or archive identifiers, observation dates, and the functional requirements inferred in an honest local provenance note. Tracked design records must explain CurveBook's own requirements and design decisions in neutral terms without embedding the frames or linking into the ignored archive.
- **Verification:** Do not infer numerical algorithms or reference answers from frames, or use them as visual matching targets, acceptance baselines, or evidence that a workflow works. Review for unintended copying may identify similarities to remove; it must not require visual similarity. Validate numerical behaviour through published scientific references and verified oracles, and validate usability through CurveBook's own requirements and tests. The existing branding, asset-provenance, licence, and human-path evidence gates remain in force.

---

## Verification

Run the hygiene gates whenever editing user-facing copy, format boundaries, assets, or policy files:

```sh
cargo xtask brands
cargo xtask cleanroom
cargo xtask assets
```
