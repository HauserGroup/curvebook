# Rename LumenCraft to CurveBook

Status: scheduled as **R0.20 — Rename LumenCraft to CurveBook**, owned by maintainers, **todo**. Complete the cutover after the landed Rust/MCP/README baseline and before further unclaimed R10 GUI implementation. This revision updates planning and sequencing only; it does not start the rename or claim its implementation. See [the roadmap](../docs/planning/roadmap-history/r0.md#r020-rename-lumencraft-to-curvebook).

## Basis and retained decisions

Reviewed on 2026-11-16 against local `main` at **`ed99e446e0283c44f362ddb5fa43e0c20814aacc`** (`ed99e44`, “Remediate R0.19 Slice S02 statistical findings (R2-S02-01..16)”). A read-only remote check returned `ac7ef26c69925d92b58ae34b8a83fde5bc95a513` for GitHub `main`, matching the local `origin/main` reference; local `main` includes newer commits. No fetch, checkout, merge, or remote mutation was performed. The inventory below uses committed local `main`, not the dirty working tree. The previously staged statistics fixes landed on `main` during this refresh and are included in this final baseline; uncommitted roadmap, audit, release, and Windows-signing work remain separate and must be preserved.

The source inventory at this commit contains 6,269 case-insensitive `lumencraft` occurrences in 517 tracked UTF-8 text files, plus 74 branded tracked file paths. This was counted from a committed `git archive` snapshot, then refreshed using the committed diff through the final baseline, excluding binary payloads and ignored/uncommitted files. These are a dated inventory snapshot, not maintained product metrics. Repeat the inventory against the implementation baseline. This plan is now the sole exception to the ignored `plan/` contents so the roadmap can link to a versioned specification.

Earlier decisions remain in force:

- **Full cutover:** no LumenCraft file readers, executable aliases, environment-variable fallbacks, settings migration, or browser-storage migration in CurveBook.
- Use `HauserGroup/curvebook` as the intended GitHub repository destination.
- Change native extensions to `.curvebook`, `.curvebook.d/`, and `.cbtemplate`.
- Give CurveBook a fresh Windows installer identity; leave LumenCraft installations separate.
- Retain the current approved seven-pane artwork, its pixel-art README variant, and the current icon geometry, colours, proportions, and alpha. The former mole artwork is now archived provenance on `main`; preserve its original file and contributor credits rather than restoring it as the active icon.
- Require an explicit `CURVEBOOK_MAINTAINER` value for Linux package formats that need a contact address; do not invent a new project email or domain.
- Keep workspace package version `0.1.0` during the rename. A release or version bump is separate work.

The project is pre-alpha with no users. Use this point for a deliberate breaking cutover before additional GUI identity, persistence, packaging, and onboarding work accumulates. The rename still touches executable and persisted interfaces and requires the verification below.

Current `main` includes these additional surfaces since the previous plan:

- **Rust baseline (`R0.18`):** Rust `1.99.1`, `rust-toolchain.toml`, CI/coverage/packaging pins, and coverage automation are landed. Keep those versions and current third-party dependency versions, including `rfd`, unchanged.
- **MCP and README (`R7.3`/`R7.4`):** actual subprocess stdio/headless acceptance, strict GUI-only policy, progressive toolsets/resources/prompts, authenticated live control in `apps/lumencraft/src/live_control.rs`, and opt-in `--control <port> --window` share the engine session. Rename the verified beginner README instructions, MCP client configuration, binaries, token/config paths, worker-thread names, and acceptance fixtures together.
- **Landed R10 work:** native `DesktopCommandRoute`/dialog routing and extended file pickers (`R10.6`); browser `text_bridge.rs`/`chrome_bridge.rs`, canvas measurement/drawing and menu/toolbar/accessibility DOM (`R10.7`); proportional text APIs (`R10.9`); external-branding and original-asset guards (`R10.14`); and two-dimensional equation layout (`R10.19`). Preserve these changes, the completed P-value/CLD engine work, and existing R10 claims. Scheduling the rename now does not undo or reclassify already-landed GUI work, nor prove human GUI completion.
- **Audit remediation (`R0.19`):** recent committed CLI/MCP/path-policy, data-transform, fitting, and statistical fixes and their regression tests are part of the baseline. Independent audit/scientific fixes may continue, but coordinate file/path ownership and integrate them before taking the rename snapshot.
- **Packaging work in progress:** the working tree plans `R8.9` MSIX/Store distribution and Windows signing guidance separately. These are not implemented MSIX capabilities on committed `main`. Preserve that work and update its identity references if it lands before the cutover; Store account setup, signing credentials, and publication remain separate maintainer actions.

The platform layer, typed registries, versioned persistence, native window, Trunk/WebGL2 host, generated schemas, and existing workflow suites are established rename surfaces. Consult source and dated test evidence when older status prose disagrees.

## Sequencing and implementation readiness

1. `R0.18`, `R7.3`, and `R7.4` form the completed prerequisite baseline.
2. Claim `R0.20` under the repository rules, coordinate existing audit/GUI claims, rebase the isolated rename work onto the then-current `main`, inventory tracked content/paths, and run baseline checks before changing identities. Preserve unrelated staged and uncommitted work.
3. Complete and verify `R0.20` as one coherent cutover, then resume remaining unclaimed `R10` implementation using CurveBook identities. Existing in-progress work must finish or be deliberately rebased with its owner; do not silently interrupt it.
4. Keep repository-host administration and local directory moves as explicit handoff tasks after the reviewed cutover. A pending host rename must be recorded accurately and is not permission to publish or change accounts.

## Canonical naming and public interfaces

| Existing identity | CurveBook identity |
|---|---|
| `LumenCraft`, including compound types such as `LumenCraftNativeApp` | `CurveBook`, `CurveBookNativeApp` |
| `lumencraft` | `curvebook` |
| `LUMENCRAFT` and `LUMENCRAFT_*` | `CURVEBOOK` and `CURVEBOOK_*` |
| `Lumencraft` in Rust/WiX product identifiers, including `LumencraftDirectory` | `CurveBook`, `CurveBookDirectory` |
| `lumenCraft` / `lumen_craft` / `LUMEN_CRAFT` if found | `curveBook` / `curve_book` / `CURVE_BOOK` according to identifier convention |
| `lumencraft-*` Cargo packages and binaries | `curvebook-*` |
| `lumencraft_*` Rust crate imports and branded identifiers | `curvebook_*` |
| Product-specific `LC`, `lc-`, `lc_` | `CB`, `cb-`, `cb_` |
| `apps/lumencraft{,-cli,-web}` | `apps/curvebook{,-cli,-web}` |
| `.lumencraft`, `.lumencraft.d/`, `.lctemplate` | `.curvebook`, `.curvebook.d/`, `.cbtemplate` |
| Native format IDs `lumencraft`, `lumencraft.d`, `lctemplate` | `curvebook`, `curvebook.d`, `cbtemplate` |
| Archive magic `lumencraft-archive` | `curvebook-archive` |
| Batch identity `lumencraft.batch.v1` and recognized version prefix | `curvebook.batch.v1`, `curvebook.batch.v` |
| MCP server name `lumencraft` | `curvebook` |
| `org.lumencraft.LumenCraft` | `org.curvebook.CurveBook` |
| `org.lumencraft.project`, `org.lumencraft.template` | `org.curvebook.project`, `org.curvebook.template` |
| `application/vnd.lumencraft.{project,template}` | `application/vnd.curvebook.{project,template}` |
| `lumencraft.lock`, `snapshot.lumencraft`, `lumencraft-data` | `curvebook.lock`, `snapshot.curvebook`, `curvebook-data` |
| `LumenCraftHost` browser bridge | `CurveBookHost` |
| `lumencraft.v1.preferences`, `lumencraft.v1.autosave.*`, `lumencraft.v1.tab_locks`, `lumencraft.doc.*` | Corresponding `curvebook.*` browser storage, channel, and lock names |
| Both `lumencraft/lumencraft` and `HauserGroup/lumencraft` repository references | `HauserGroup/curvebook` |
| Project-owned schema IDs under `https://lumencraft.dev/schemas/` | `https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/<filename>` |

Use repository-backed JSON Schema identifiers so the plan does not assume ownership of a CurveBook domain. They are identifiers, not runtime network dependencies. Preserve standard JSON Schema vocabulary URIs and third-party source URLs.

Review abbreviations in context: do not replace scientific terminology, dependency names, or unrelated uses of `LC`. Preserve generic crate directories, command IDs, user data, and non-branded filenames such as `manifest.json`, `document.json`, `ops.journal`, and `portable.txt`.

## Implementation work packages

### 1. Workspace, registries, and guard coverage

- Rename all product-prefixed workspace packages, including `lumencraft-platform`, and the application directories/binaries. Update Cargo dependency keys, paths, feature references, build scripts, crate imports, Rust types/functions, tests, doctests, and CLI package-selection examples.
- Refresh `Cargo.lock` without upgrading dependencies, adding crates, changing allowed licences, or replacing lockfile sources. Preserve Rust `1.99.1`, the rustup pin, Cargo features, native targets, `web-release`, and `wasm32` settings. Inventory case-insensitive and split-word brand variants, then use context-aware replacements in descending specificity; generic case folding must not damage Rust or installer identifier casing.
- Update `xtask` layer classification, default application selections, build/bundle helpers, corpus/font tooling, generated-document inputs, and tests that contain package names or paths.
- Update path-specific telemetry and security checks with the directory move, particularly `xtask/src/telemetry.rs` loopback allowlists, web-source roots, and required HTML location. Update the expanded `xtask/src/brands.rs` UI/window/HTML/SVG/PDF/manifest checks, original-asset provenance rules in `xtask/src/cleanroom.rs`, and coverage defaults/exclusions in `xtask/src/coverage.rs`. Ensure guards and regressions still inspect the renamed applications rather than silently skipping them; retain their external-branding restrictions.
- Preserve the canonical typed `CommandSpec` and `UiCommandSpec` registries, CLI/MCP tool schemas, GUI-only policy, command effects, and replay behavior. Do not rename stable domain command IDs or remove neutral legacy command aliases as part of the product rename.
- Rename all implemented branded environment variables and matching documentation, including `LUMENCRAFT_ALLOW_OS_GUI_DIALOG`, control/config settings, build SHA/date/version, fonts, corpus controls, and Windows installer properties. Inventory actual readers separately from names that appear only in documentation; do not add missing features under the rename.

### 2. Native formats, settings, and recovery

- Apply the new identities through the typed `FormatRegistry`, importer/exporter descriptors, archive validator, native extension constants, CLI format parsing, and generated schema source definitions in the document and format crates.
- Update both the CLI batch schema declaration and the shared versioned-file validator, including the version-prefix logic for refusing unsupported future `curvebook.batch.vN` manifests.
- Treat CurveBook as a new named format family. Preserve document structure, numerical schema version, migration semantics within the new family, and preferences/journal/lock versions where their payload structure is unchanged. Do not introduce a legacy-to-CurveBook migration or silently relabel old archives. Keep the existing future-version refusal and no-overwrite guarantees.
- Rename recovery lock/snapshot names, process-owner labels, temporary paths, desktop application-support directories, Unix config/data directories, portable storage, and all matching tests. Continue using the platform crate's atomic writes and bounded OS helpers.
- Preserve checkpoint, unpacked-directory, import-link, and template functionality already present on `main`, including deferred QoL implementations. Deferral is not authorization to delete existing file support.
- Reject old native archive magic and old batch identities explicitly. Test filename dispatch and content-sniffing/importer fallbacks so an old native archive cannot be misidentified as a supported interchange project after renaming its suffix.
- Leave users' existing files, old settings directories, old recovery journals, browser storage, and installed applications untouched. Explain that changing a filename extension alone does not convert the archive. Any conversion utility or migration service is outside this plan.

### 3. Live desktop and browser surfaces

- Rename the native GUI application type, window titles, `eframe` application identity, branded egui widget/modal/texture/font keys, `rfd` open/save/import/export/template descriptions and filters, default filenames, and OS clipboard/print metadata. Cover `DesktopCommandRoute`/`DesktopDialogOpener` and `live_control.rs` worker names/token lifecycle, preserving authentication, shared-session behavior, typed command routing, and file-policy checks. Update native host/control tests as well as headless CLI tests.
- Update browser HTML title/ARIA labels, DOM IDs and CSS selectors, loading/drop/read-only/print surfaces, archive/directory picker filters, download names, and the Rust WASM entry point together. Cover the live `text_bridge.rs` and `chrome_bridge.rs` paths, HTML accessibility controls, `WebChromeBridge` native-format enums/tags, browser measurement/draw callbacks, menu/toolbar routing, and their host adapter tests. Keep DOM, Rust callbacks, file tags, and bridge names aligned.
- Rename both sides of the existing browser bridge (`window.CurveBookHost` and the Rust lookup), exported branded methods, native-format tags, localStorage keys/autosave prefixes, broadcast-channel names, and Web Lock names. Keep Rust adapters and the existing host glue synchronized; do not expand or rearchitect the glue during the rename.
- Use fresh CurveBook browser namespaces without reading, deleting, or copying the old namespace. Verify tab contention and autosave recovery still work between CurveBook tabs.
- Preserve relative deployment URLs, `+simd128`, Trunk/WebGL2 and 2D fallback behavior, private local data processing, strict CSP/COOP/COEP headers, and zero-egress checks. Do not add an updater or runtime network lookup.
- Update localized About/menu text, help/version output, result-report branding, PDF creator/producer fields, generated fallback-font names, GUI inspection text, MCP instructions, and branded error/log prefixes. Scientific results and diagnostic policy remain unchanged.

### 4. Assets, packaging, and repository metadata

- Rename icon files and platform icon IDs without redrawing or re-encoding unchanged artwork. Cover the current seven-pane platform exports, `lumencraft-win95-2508.png` README image, the associated original source/prompt records, and archived mole attribution. Update runtime resource references, README image links, icon-generation scripts, `ASSETS.md`, and `ATTRIBUTION.md` while preserving original provenance and licences. Use the configured VectorCraft MCP for any actual SVG-content inspection/edit/export task; pathname-only moves and reference updates do not authorize an artwork redesign.
- Regenerate the existing software-rendered workspace screenshots when visible branding changes, using the repository's synthetic datasets and screenshot generator; update attribution as needed. Do not use screenshots from proprietary software or real user projects.
- Update macOS `.app` names, bundle identifiers, executable/icon references, plist metadata, exported UTIs, document handlers, and native launch identity together.
- Update Linux/FreeBSD package slugs, AppImage/AppRun/update metadata, desktop and AppStream entries, MIME definitions, Flatpak IDs/manifests, OCI image names, icon paths, documentation paths, and packaging validators.
- Update Windows resource metadata, WiX filenames/identifiers/localization, shortcuts, registry searches/keys, install paths, binary payload names, signing descriptions, and portable archives. Generate fresh upgrade and explicitly assigned component GUIDs once; pin them in the policy and update identity-guard tests without weakening dual-scope/folder protections. Do not uninstall or upgrade LumenCraft as a side effect.
- Remove the Linux maintainer-email fallback and require `CURVEBOOK_MAINTAINER` only when a selected package format requires it. Fail early with a clear error for those formats; other archive builds must remain usable without an invented address.
- Normalize Cargo repository metadata, badges, issue/security links, release links, signing URLs, contributors-generator defaults, and hosting metadata to `HauserGroup/curvebook`.
- Update `.github` templates/workflows (including coverage and platform-specific jobs) and packaging scripts only where names, paths, metadata, cache/artifact keys, or variables change. Preserve current triggers and Rust pins; do not invoke publishing paths for rename verification. Check newly landed MSIX manifests, CLI execution aliases, signing instructions, and release artefact names if `R8.9` implementation exists by then. Do not invent Partner Center identities or turn this rename into Store distribution work.
- Rename project branding in `about.hbs`, README, contributor/security/governance docs, notices, and original-asset licence notes. Preserve all third-party notices, human contributor identities, copyright years, and licence text. Never hand-edit generated contributor records or opt-in human names.

### 5. Generated outputs and documentation

- Change generator sources first, then run `cargo xtask ui-commands`, `cargo xtask commands`, `cargo xtask schemas`, and `cargo xtask contributors`. Do not hand-edit generated UI catalogs, command documentation, persisted JSON Schema snapshots, or contributor JSON.
- Regenerate the third-party licence report through the existing `cargo about`/`cargo xtask licenses --html` release tooling after updating the branded template; preserve all third-party disclosures.
- During implementation, update `docs/ROADMAP.md`, `docs/STATUS.md`, gaps/parity evidence, command/file-format/development/release/signing guides, and internal anchors in the same rename PR. Keep the minimal README and accessible, verified MCP walkthrough intact under the new binary/package names. Use current generated/gate-checked metrics rather than copying old counts. This planning revision changes only the rename specification, its tracking rule, and roadmap/status sequencing.
- Preserve current QoL decisions: diagnostics-only item 7; deferred 10, 11, 13, and 15; equations-only 16; the extended GUI workflow coverage already present. A product rename must neither reopen deferred work nor change statistical behavior.

## Verification and acceptance

### Source and functional verification

- Verify Cargo metadata has only the new workspace identities; review the lockfile diff for internal package renames with no third-party version/source changes.
- Run the renamed workspace's `cargo xtask ci`. Also verify applicable current gates: `ui-commands --check`, `commands --check`, `schemas --check`, `screenshots --check`, `i18n-report`, `layers`, `assets`, `brands`, `cleanroom`, `secrets`, `licenses`, `telemetry`, `workflows`, `status`, and `pending`.
- Verify native-default, optional bundled-font, and no-default-feature paths using the new font environment variables; keep tracked font binaries absent. Build the actual WASM target and Trunk `web-release` bundle, rather than relying solely on host adapter tests.
- Extend existing format/versioned/seam tests for new format IDs, archived and unpacked round trips, templates, recovery, future-version refusal, and explicit rejection of legacy names/magic/schema identities. Build negative inputs in tests; do not commit legacy or third-party binary fixtures.
- Smoke-test `curvebook` and `curvebook-cli` help/version output, save/open/convert/batch flows, and the renamed README setup. Re-run actual spawned stdio/headless `R7.3` workflows and protocol/toolsets/resources/prompts/path-policy checks, plus authenticated `--control <port> --window` shared-session and strict GUI-only tests. Verify token cleanup and new environment names without old aliases. Keep existing semantic GUI suites and landed native/browser routing, text, equation, and audit regressions passing; numerical oracle tolerances and scientific results must remain unchanged.
- Exercise the actual desktop GUI: title, icon, open/save dialogs, unsaved-close handling, portable settings, recovery, and new app identity. Check branding in localized About views and generated methods/PDF output.
- Exercise the built browser: WASM startup and host bridge, new file filters/downloads, settings/autosave namespaces, same-document contention between tabs, dropped files/directories, print output, and relative-path hosting. Confirm old storage is untouched and there is no bridge-name mismatch.

### Package and residual-name verification

- Run `packaging/check-app-specific.sh`, Windows icon and installer-policy checks, macOS bundle/plist/codesign and `verify.sh`, and Linux AppRun validation on their supported hosts. Build and inspect the existing platform packages and browser archive without publishing them.
- Check artifact filenames and internal paths, metadata, file associations, app IDs, icon resources, licence/contributor disclosures, and the new Windows identity. Verify fresh CurveBook installation does not claim old native extensions or modify LumenCraft installation/settings state.
- Audit every tracked text file and tracked path case-insensitively, including hidden CI files, generated outputs, and contextual product abbreviations. Record a narrow residual-name allowlist for this before/after specification, focused rejection tests, breaking-change explanations, and exact historical commands/artifact names needed as audit provenance. Preserve historical commit hashes and scientific citations; update navigational links to renamed live files. Residual old names must not create compatibility behavior or become current product instructions.
- Audit regenerated screenshots and packaged/generated text for visible old branding. Exclude Git history, ignored build caches, external corpora, user-owned files, and this planning notebook from bulk replacement. Never perform byte replacement inside PNG/ICO/ICNS files or compiled artifacts.
- Report only checks actually run. Record environmental or target-OS limitations explicitly; a green headless suite does not establish live desktop/browser or cross-OS packaging readiness.

## Delivery boundaries

1. This revision schedules implementation as **R0.20** but does not execute it. At implementation start, reread current governance/domain guides, inspect issues/open PRs and recent `main` commits, coordinate overlapping claims, and claim the item before branching. Use an isolated checkout and the current branch/PR convention; this Codex environment uses `codex/R0.20-curvebook-rename`, with a PR title ending in `(R0.20)`.
2. Recheck the tree and baseline before editing. The inspection working tree was not clean; preserve all staged/uncommitted work and incorporate only reviewed changes from the implementation baseline. Keep unrelated feature, dependency, numerical, and CI changes out of the rename PR.
3. Complete the rename and its generated outputs as one coherent reviewable PR. Keep implementation changes off `main` until reviewed and merged under repository policy. Leave `R0.20` todo until claimed and report only verified completion, with target-OS or external handoff limits explicit.
4. After a concrete reviewed result exists, separately approve and carry out the GitHub repository rename and origin URL change. Recheck links, Actions, signing/hosting configuration, and release destinations; do not assume redirects cover package assets or integrations.
5. Rename the local checkout directory and reopen project/worktree references only after active sessions finish. Repository-host administration, secrets/account changes, publication, deployments, tag pushes, release creation, and replacing old published artifacts are separate authorized actions.
6. Preserve existing Git history, published historical artifacts, human attributions, current dependencies/licences, clean-room rules, no-panic behavior, atomic persistence, and zero-telemetry safeguards. This adds roadmap sequencing only: no calendar automation, new dependency, asset redesign, migration tool, feature rollout, publication, or account action is included.
