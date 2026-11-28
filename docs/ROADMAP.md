# CurveBook Roadmap

Claimable engineering roadmap for **CurveBook**. For verified capabilities, live counts, and known limits, see [`STATUS.md`](STATUS.md). Completed items and their verification records live in the [archive index](#completed-roadmap-items) and [Roadmap History](history/roadmap.md).

**Baseline (`2026-11-27`):** Rust `1.99.1` (`edition = "2024"`, [`R0.18`](history/roadmap/r0.md#r018-upgrade-the-workspace-and-build-toolchains-to-rust-1991)), CurveBook identity (`R0.20`), `v0.1.0-alpha.1`, the declarative `egui 0.37` desktop GUI (`R15`, `R20`), `65 / 65` five-stage desktop GUI workflows (`R10.25`–`R10.30`, `R13`), `.pzfx`/`.prism` interop & schema `v9` project merging (`R22`), live recompute of corpus-verified `.prism` analyses (`R25.1`), and local `GAP-01`–`GAP-07` / delta-audit fixes (`R0.36`) are complete locally. Remaining **Beta** work is purely hosted CI/release/Pages verification (`R0.27`, `R0.36`, `R12.1`, `R14.1`); post-Beta and **1.0** items follow below.

---

## How to use this file

1. **Pick the next item** from [Order and dependencies](#order-and-dependencies) whose dependencies are `done` and whose gate matches the current release target.
2. **Claim before branching:** mark `Status: in progress — claimed: <handle>, branch <branch-name>` (and later `— in PR #n`) in a small claim commit/PR, and read [`AGENTS.md`](../AGENTS.md), [`pitfalls.md`](pitfalls.md), and [`extending.md`](extending.md) first.
3. **One item per PR:** scope PRs to one `R<m>.<n>` item, use branch prefixes `feature/`, `bugfix/`, `hotfix/`, `refactor/`, `docs/`, `test/`, `chore/`, or `audit/` with the ID, and end squash PR titles with `(R<m>.<n>)`. Issue fixes outside planned work use `bugfix/issue-<N>-<slug>` and `Fixes #<N>` ([`AGENTS.md`](../AGENTS.md) › Git Workflow).
4. **Metadata, checkboxes & archiving:**
   - Headers use `### R<m>.<n> <title>` followed by `Owner: maintainers|open · Status: todo|in progress|blocked (why)|done|merged into R<m>.<n> (<date>)|dropped (<date>, why) · Was: <notes>`.
   - `- [ ]` lines track concrete pending sub-tasks (parsed by `cargo xtask pending`). Mark `- [x]` only after verification passes.
   - Move completed, merged, or dropped items to their milestone file under [`docs/history/roadmap/`](history/roadmap/), link them in the [archive index](#completed-roadmap-items), and update inbound links.
   - Hour estimates are single-agent notes only (excluding human review).
5. **Unique table keys:** every Markdown table in `docs/ROADMAP.md` and `docs/STATUS.md` MUST have unique Column 1 values (`cargo xtask status`).

## Architecture rules for new work

All roadmap work follows [`AGENTS.md`](../AGENTS.md) §3 (headless-first `L0`–`L5` engine mutations, declarative seam registration per [`extending.md`](extending.md), structured `EngineError`/`DocError`/`FormatError` returns, versioned files with forward-refusal per [`file-formats.md`](file-formats.md), and atomic writes/checked locks/bounded subprocesses via `curvebook-platform`).

---

## Milestones at a glance

| Milestone | Target gate | Status | Open items / Archive |
|---|---|---|---|
| **R0** Align with house principles | Beta / Hygiene | in progress | Open: [`R0.16`](#r016-local-source-coverage-reports-cargo-llvm-cov), [`R0.27`](#r027-user-first-readme-overhaul-beta-gate), [`R0.35`](#r035-keep-the-ci-cache-under-the-repository-budget), [`R0.36`](#r036-full-workspace-gap--delta-audit-remediation-beta-gate). Archived: [`r0.md`](history/roadmap/r0.md). |
| **R1**–**R7** Core math, tables, stats, fit, charts, MV, automation | Alpha / 1.0 | done | All items verified & archived ([`r1.md`](history/roadmap/r1.md)–[`r7.md`](history/roadmap/r7.md)). |
| **R8** Desktop/WASM shell, localization & `1.0` hardening | 1.0 / Store | in progress | Open: [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening), [`R8.9`](#r89-windows-msix-packaging--microsoft-store-distribution), [`R8.11`](#r811-windows-authenticode-signing-after-beta), [`R8.12`](#r812-desktop-rtl-gui-mirroring--native-speaker-translation-review-10-gate) (`ar`/`he` RTL unlikely), [`R8.13`](#r813-older-macos-metal-device-lost-hardening--windows-console-suppression). Archived: [`r8.md`](history/roadmap/r8.md). |
| **R9** Scientific quality-of-life suite | Deferred | done (active scope) | Active `QOL` items archived ([`r9.md`](history/roadmap/r9.md)); `R9.10`, `R9.11`, `R9.13`, `R9.15` deferred ([scope](planning/qol-enhancements.md)). |
| **R10** Human-operable GUI & `65`-workflow completion | Beta (done) / 1.0 | in progress | `65/65` automated workflows, `R10.29`–`R10.30`, `R10.34`–`R10.36` archived ([`r10.md`](history/roadmap/r10.md)). Open: [`R10.13`](#r1013-human-operable-50-workflow-acceptance--manual-qa), [`R10.24`](#r1024-windows--linux-desktop-gui-smoke-qa-10-gate) (**1.0**); [`R10.31`](#r1031-desktop-graph-image-gallery--linked-region-ui-parity-fixes)–[`R10.33`](#r1033-advanced-importpaste-special-live-link-persistence--export-figure-options) (post-Beta). |
| **R11** Usable browser app (`eframe::WebRunner`) | 1.0 | in progress | `apps/curvebook-web` is a strict-CSP placeholder until [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate); browser tests & QA (`R11.1`–`R11.10`) deferred to **1.0**. |
| **R12** User documentation, Pages site & workflow guides | Beta / post-Beta / 1.0 | in progress | Open: [`R12.1`](#r121-documentation-site-on-github-pages) (**Beta** live check); [`R12.2`](#r122-example-workflow-guides), [`R12.3`](#r123-functionality-and-methods-reference), [`R12.7`](#r127-application-user-guide-desktop-and-browser), [`R12.8`](#r128-search-discoverability-for-the-pages-site). Archived: [`r12.md`](history/roadmap/r12.md). |
| **R13** Advanced regression, graphing, layouts & `WF-51`–`WF-65` | Beta | done | `R13.1`–`R13.10` archived ([`r13.md`](history/roadmap/r13.md)). |
| **R14** Releases & distribution | Beta / post-Beta | in progress | Open: [`R14.1`](#r141-beta-release-pipeline) (**Beta**); [`R14.2`](#r142-release-sbom-and-ci-runner-hardening)–[`R14.5`](#r145-mcp-bundle-mcpb-and-mcp-registry-listing-post-beta), [`R14.7`](#r147-software-citation-citationcff-zenodo-doi--versioned-methods-paragraph-post-beta). Archived: [`r14.md`](history/roadmap/r14.md). |
| **R15** Modern `egui` desktop GUI overhaul | Beta | done | `R15.1`–`R15.10` archived ([`r15.md`](history/roadmap/r15.md)). |
| **R16** Core- and GUI-crate maintainability | Deferred | done (active scope) | [`R16.1`](#r161-split-the-stats-anova-module-by-method-family) Tier 1 done (optional Tier 2 splits deferred); `R16.2`–`R16.19` archived ([`r16.md`](history/roadmap/r16.md)). |
| **R17** Post-Beta adoption, interop & real-world polish | post-Beta | in progress | Open (external/hardware QA only): [`R17.2`](#r172-downstream-vector-and-document-export-interoperability)–[`R17.4`](#r174-live-llm-mcp-usability-trials-and-ergonomics-fixes). `R17.1`, `R17.5`–`R17.7` archived ([`r17.md`](history/roadmap/r17.md)). |
| **R19** Statistical, fitting & `D1` oracle breadth | post-Beta | done | `R19.1`–`R19.7` archived ([`r19.md`](history/roadmap/r19.md)). |
| **R20** Desktop GUI visual system & layout overhaul | Beta / post-Beta | done | `R20.1`–`R20.22` archived ([`r20.md`](history/roadmap/r20.md)). |
| **R21** Structural LOC, test-layer & docs reduction | none | done | `R21.1`–`R21.4` archived ([`r21.md`](history/roadmap/r21.md)). |
| **R22** `.pzfx`/`.prism` compatibility & project merging | Beta | done | `R22.1`–`R22.6` landed in `03c7b15` & archived ([`r22.md`](history/roadmap/r22.md)). |
| **R23** Analytical, statistical, regression & formula breadth | Optional before Beta | in progress | Open: [`R23.2`](#r232-complete-lognormal-hypothesis-testing-suite--standalone-normalitylognormality)–[`R23.6`](#r236-nonlinear-regression-constraints-derived-ci-transforms-equation-catalog-dsl-depth--theoretical-plotting-62) (non-blocking). `R23.1` archived ([`r23.md`](history/roadmap/r23.md)). |
| **R24** Advanced graph formatting, pipelines, layouts & visual audit | post-Beta (deferred) | todo | [`R24.1`](#r241-advanced-graph-formatting-axes-scale-bars-heatmaps--multivariate-visuals-post-beta)–[`R24.3`](#r243-workflow-specification-backlog--full-visual-parity-audit-post-beta). |
| **R25** Imported analysis live recompute & interop accuracy | Beta (done) / post-Beta | in progress | [`R25.1`](#r251-promote-corpus-verified-imported-analyses-to-live-recompute-beta-gate) (**Beta**, done locally). Open: [`R25.2`](#r252-imported-analysis-accuracy-gaps-post-beta)–[`R25.4`](#r254-interop-follow-ups-saving-rebuilt-graphs--post-test-cost-post-beta) (post-Beta, tracked in GitHub issues). |

**Gate summary:**
- **Alpha:** Released 2026-11-17 as [`v0.1.0-alpha.1`](https://github.com/HauserGroup/curvebook/releases/tag/v0.1.0-alpha.1).
- **Beta (automated only; no manual human step required):** All local Beta code is complete, including desktop GUI (`65/65` five-stage `WorkflowRun`), `.pzfx`/`.prism` interop & schema `v9` project merging (`R22`), live recompute of corpus-verified imported `.prism` analyses ([`R25.1`](#r251-promote-corpus-verified-imported-analyses-to-live-recompute-beta-gate), done locally), and `GAP-01`–`GAP-07` / delta-audit fixes (`R0.36`). Remaining Beta gates are hosted-only: hosted `ci.yml` & `release.yml` dry-run/rehearsal checks ([`R14.1`](#r141-beta-release-pipeline), [`R0.36`](#r036-full-workspace-gap--delta-audit-remediation-beta-gate)), scripted README rehearsal check ([`R0.27`](#r027-user-first-readme-overhaul-beta-gate)), and automated live Pages verification ([`R12.1`](#r121-documentation-site-on-github-pages)). Browser app (`R11`) is deferred to **1.0**; desktop RTL (`ar`/`he`) support is **unlikely** and unscheduled.
- **1.0:** Recorded desktop/browser walkthroughs ([`R10.13`](#r1013-human-operable-50-workflow-acceptance--manual-qa)), Windows/Linux smoke & VM QA ([`R10.24`](#r1024-windows--linux-desktop-gui-smoke-qa-10-gate), [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening)), native-speaker review of shipped LTR/CJK `[Draft]` translations ([`R8.12`](#r812-desktop-rtl-gui-mirroring--native-speaker-translation-review-10-gate)), Windows Authenticode signing ([`R8.11`](#r811-windows-authenticode-signing-after-beta)), and `eframe::WebRunner` browser app ([`R11`](#r11-usable-browser-app)).

> **Retired `gui_only` note:** Historical `gui_wf_*` tests (retired under `R21.1`) drove semantic widget patches rather than drawn widgets. Beta's GUI gate (`R10.25`) counts only five-stage `EguiHumanPathDriver` runs (`human_wf_*`).

---

## R0 Align with house principles

### R0.16 Local source-coverage reports (`cargo-llvm-cov`)
Owner: open · Status: in progress · Was: coverage tooling review 2026-11-15 · Est. note: 2–4 h

Coverage is a diagnostic for untested edge-case and error paths (never a percentage gate or substitute for `D1` oracles).

- [x] Added `cargo xtask coverage` (`xtask/src/coverage.rs`, `5` unit tests), weekly/manual `.github/workflows/coverage.yml` (artifact-only, zero external uploads), and policy docs in [`development.md`](development.md) and [`AGENTS.md`](../AGENTS.md) §3.
- [ ] Confirm a green hosted run of `coverage.yml` (`workflow_dispatch`) once it is on `main`.

### R0.27 User-first README overhaul (Beta gate)
Owner: open · Status: in progress · Was: Alpha release follow-up 2026-11-17 · Depends on: [`R14.1`](#r141-beta-release-pipeline), [`R12.5`](history/roadmap/r12.md#r125-getting-started-files-settings-and-support) · Target gate: **Beta** · Est. note: 6–10 h

- [x] Rewrote [`README.md`](../README.md) for end users with zero hardcoded mutable counts, moved contributor/MCP detail into [`development.md`](development.md), [`mcp.md`](mcp.md), and [`CONTRIBUTING.md`](../CONTRIBUTING.md), and linked single-home install (`get-started.html`) and known-issues (`troubleshooting.html#known-issues`) pages (2026-11-18).
- [ ] Verify: rerun every command and install step against the `R14.1` rehearsal assets on hosted macOS, Windows, and Linux runners as a scripted job; check links and anchors; pass `brands`, `cleanroom`, `assets`, and `status`. No manual step.
- [ ] **1.0 (moved from Beta 2026-11-24, because it needs a person):** record a walkthrough by a first-time reader who is not a developer.

### R0.35 Keep the CI cache under the repository budget
Owner: open · Status: in progress · Was: hosted CI and release review 2026-11-21 · Depends on: none · Gate: none · Est. note: 6–10 h

- [x] Landed `cargo xtask ci-cache {trim,restore-mtimes}` (`xtask/src/ci_cache.rs`), rolling per-job/OS cache keys with `prune-caches` in `ci.yml`, and `save-if: ${{ github.event_name != 'schedule' }}` in `release.yml` (`71f2037`).
- [ ] Verify: on `main`, after a week of runs, the cache stays under the budget with room to spare and every platform cache is still present; record the before and after sizes and the `tests` stage time.

### R0.36 Full-workspace gap & delta audit remediation (Beta gate)
Owner: maintainers · Status: in progress (all local GAP-01..GAP-07, six-slice delta audit, Cividis LUT & follow-up remediation landed on `main`; hosted CI & release dry run open) · Was: Tier-2 & delta gap audits 2026-11-25..2026-11-26 (`GAP-01`–`GAP-07` + `b298cb7` + `638cfd4`) · Depends on: none · Gate: **Beta** · Est. note: 0 h local remaining

- [x] Closed all `5` `S1`, `12` `S2`, and `S3`/`S4` Tier-2 gap-audit findings (`14b823d`, `eed57fa`) plus the six-slice delta audit across desktop, UI, charts, numerics, platform, CLI, release, and docs (`b298cb7`), verified locally with full regression suites.
- [x] **Cividis stops (decision):** switched `CIVIDIS_STOPS` to the exact 17 published `viridisLite` 0.4.2 option `"E"` (Nuñez, Anderton & Renslow 2018) LUT samples matching `VIRIDIS_STOPS` and `PLASMA_STOPS`, anchored all 5 quarter-point LUT entries, and widened the CIE ΔL* smoothness band to `[3.6, 6.4]` for the published `6.27`/`3.70` first two intervals (`638cfd4`).
- [x] **Delta audit follow-ups (non-blocking):** added `AxisSegment::auto_log_minor_ticks` (`#[serde(default = "default_true")]`) so explicit `minor_ticks_per_interval: 0` turns Log10 minor ticks off while preserving `2..=9 × 10^k` defaults; extended modal-open guards to File › Open `.curvebook` / unpacked / template / merge entry points; surfaced Export All batch errors and non-Welcome-hub `status_banner` notices via a dismissible `status.banner_notice` footer chip plus single-surface desktop toast adoption and leftover-file warnings; and isolated `r15_2_column_grid.rs` grid tests onto per-test `StubPlatformClipboardAdapter` instances (`638cfd4`).
- [ ] Confirm green hosted `ci.yml` on `main` and one hosted `release.yml` dry run verifying the separated macOS/Windows signing steps (`GAP-05-02`) before Beta.

---

## R8 Desktop & WebAssembly live shell, localization & `1.0` hardening

### R8.5 Cross-OS hardware, CJK/RTL desktop QA & installer/signing hardening
Owner: open · Status: in progress · Was: G13, M7 (**1.0**)

Windows MSIX/Store distribution lives in [`R8.9`](#r89-windows-msix-packaging--microsoft-store-distribution); Apple signing is in [`R14.1`](#r141-beta-release-pipeline); Windows Authenticode signing is in [`R8.11`](#r811-windows-authenticode-signing-after-beta).

- [x] Verified local macOS universal `.dmg` packaging/validation (`plutil`, `codesign`, `verify.sh`, `check-app-specific.sh`), keyless Sigstore build-provenance attestations (`actions/attest-build-provenance` with `--signer-workflow`), and active `Release tags` / immutable-releases GitHub settings (2026-11-18).
- [ ] Complete Windows VM and Linux X11/Wayland VM (AppImage, the only Linux format after [`R0.28`](history/roadmap/r0.md#r028-rein-in-the-number-of-release-install-files)) QA runs (live browser QA is [`R11.10`](#r1110-live-browser-qa--supported-browser-matrix); FreeBSD is Tier 2 CLI-only with no desktop GUI QA). Apple Developer ID signing and notarization moved to [`R14.1`](#r141-beta-release-pipeline); Windows code signing moved to [`R8.11`](#r811-windows-authenticode-signing-after-beta) (after Beta).

### R8.9 Windows MSIX packaging & Microsoft Store distribution
Owner: maintainers · Status: in progress · Was: Windows distribution planning (2026-11-16)

Separate distribution target (not a general **1.0** gate). Store identity (`JakobSture.CurveBook`, `packaging/windows/store-identity.json`) is registered, hosted x64 Store package construction passed ([run `44301984148`](https://github.com/HauserGroup/curvebook/actions/runs/44301984148)), and [`docs/privacy.html`](privacy.html) is ready locally; no local Windows machine is available, so builds and checks run on GitHub-hosted Windows runners.

- [x] Documented Store vs. local self-signing ([`windows-signing.md`](release/windows-signing.md)), recorded Partner Center identity (`packaging/windows/store-identity.json`), verified hosted x64 Store MSIX construction (`MakeAppx` in run `44301984148`), and wrote the dedicated privacy policy ([`privacy.html`](privacy.html)).
- [ ] Add Rust `xtask` MSIX packaging using Windows SDK tools, with bounded subprocesses and atomic output publication through `curvebook-platform`; stage desktop and CLI binaries, existing attributed icons, and licence notices without bundled fonts. Keep the MSI outputs available; add architecture-specific MSIX outputs and an optional bundle. Initial `cargo xtask msix` and the x64 `Windows Store package` workflow are implemented; x64 package construction has passed on Windows; additional architecture builds and bundling remain to verify/complete.
- [ ] Define the desktop package manifest with Partner Center identity values, a stable publisher, supported Windows versions, Store-compatible version mapping, required desktop capabilities, document associations at the packaging boundary, and a tested CLI execution alias for MCP clients. Keep development identity separate when needed; do not invent Store identity values.
- [ ] Provide distinct Store-submission and test signing modes: no Azure/CA signing prerequisite for Store packages, and opt-in self-signing for test packages. Add a manual `workflow_dispatch` Windows test job that creates an ephemeral certificate on the runner, explicitly trusts its public certificate only for the disposable MSIX test session, and uploads packages, the public `.cer`, thumbprint, and test reports. Never publish private keys/PFX files; decide separately how a stable test key is retained if needed across runs.
- [ ] Verify MSIX on GitHub-hosted Windows runners: manifest validation, signature and publisher matching, installation/launch, CLI/MCP invocation, document open/save/export, upgrade, uninstall, and coexistence or migration with MSI installs. Record results for supported architectures and check that application data stays outside the read-only package directory. Run Windows App Certification Kit where supported; record runner/session limitations and arrange remote interactive Windows QA for visual/hardware checks that CI cannot establish.
- [ ] Add MSIX build/validation artefacts and checksums to release CI, plus browser-based instructions to dispatch the Windows job and download its artefacts without a local Windows PC. Keep test signing separate from public release publication, credentials out of logs, and application behavior free of egress. Updates are delivered by the Store, with no embedded updater or telemetry SDK.
- [ ] Complete Partner Center registration, reserve the product name, obtain the package identity, prepare listing/screenshots/privacy disclosures (linking the privacy policy above), submit for certification, and record the published Store URL and a verified Store update. Account setup and publication require maintainer action.

### R8.11 Windows Authenticode signing (after Beta)
Owner: maintainers · Status: in progress · Was: split from `R8.10` 2026-11-18 · Depends on: maintainer Windows signing account; [`R14.1`](#r141-beta-release-pipeline) · Target gate: **1.0** · Est. note: 4–8 h engineering, plus account setup

Beta ships unsigned Windows installers (no `WINDOWS_CERTIFICATE*` / `AZURE_*` secrets exist yet).

- [x] Guarded Windows signing steps in [`.github/workflows/release.yml`](../.github/workflows/release.yml) with `github.event_name != 'schedule'` (2026-11-18).
- [ ] Create the protected `release` environment with required reviewers, move the Apple credentials into it, and add the Windows credentials there (never as repository-wide secrets). Do this when the Windows credentials arrive, when a non-admin collaborator gets write access, or before the stable `1.0` release, whichever comes first. Never commit keys or certificates. *(Deferred from Beta on 2026-11-18: every collaborator is currently an admin who can bypass environment rules.)* *(from R8.10)*
- [ ] Maintainer: set up Windows code signing (Azure Trusted Signing or an equivalent CA certificate with a hardware- or cloud-held key). Prefer workload identity federation (OIDC) scoped to the protected `release` environment over a stored Azure client secret.
- [ ] Add the Windows signing credentials to the protected `release` environment (never repository-wide secrets) and use that environment in the `windows` job for non-scheduled, non-Alpha runs.
- [ ] Windows: Authenticode-sign `curvebook.exe`, `curvebook-cli.exe`, and every `.msi` (`x64`, `arm64`; `x86` is dropped under [`R0.28`](history/roadmap/r0.md#r028-rein-in-the-number-of-release-install-files)) with RFC 3161 timestamps. CI verifies every signature (`signtool verify /pa` or `Get-AuthenticodeSignature`).
- [ ] Fail closed for Windows: once this lands, every non-Alpha release run stops with an error when Windows signing credentials are missing instead of producing unsigned output.
- [ ] Verify on clean Windows 11 `x64` and `arm64` machines (the `R8.5` VMs) that the signed `.msi` shows the CurveBook publisher, and record SmartScreen and Smart App Control behaviour. Expect SmartScreen warnings until the signature builds download reputation; do not claim a warning-free install until one is observed.
- [ ] Update [`releasing.md`](release/releasing.md), the [`R12.5`](history/roadmap/r12.md#r125-getting-started-files-settings-and-support) install page, and the release-notes template to remove the unsigned-Windows instructions.

### R8.12 Desktop RTL GUI mirroring & native-speaker translation review (1.0 gate)
Owner: open · Status: todo · Was: localization & GUI spec audit 2026-11-24; RTL scope revised 2026-11-27 · Depends on: [`R15.5`](history/roadmap/r15.md#r155-default-desktop-egui-cutover-screenshot-refresh-and-beta-readiness) (done) · Gate: **1.0** · Est. note: external native-speaker review

**RTL scope note (2026-11-27):** Arabic (`ar`) and Hebrew (`he`) desktop GUI / full RTL chrome mirroring support (along with `fa` and `ur`) is **unlikely** and unscheduled — it is not a committed **1.0** deliverable. Existing `ar`/`he` columns in `catalog.tsv` remain hidden in `Preferences` / **View › Language** behind the `"en"` fallback in `normalize_beta_gui_locale` (`crates/ui-egui/src/i18n.rs`) unless RTL support is explicitly revisited later. The active **1.0** localization gate is native-speaker sign-off of the `13` shipped non-English LTR/CJK `[Draft]` locales.

- [ ] Complete native-speaker review of the `13` shipped non-English LTR/CJK machine-drafted `[Draft]` locales in `catalog.tsv` (`de`, `es`, `fr`, `it`, `nl`, `pl`, `pt-BR`, `ru`, `tr`, `ja`, `ko`, `zh-CN`, `zh-TW`), apply terminology/scientific-register corrections, and remove `[Draft]` markers as each locale is signed off.
- [ ] Update [`docs/evidence/localization.md`](evidence/localization.md) and [`docs/reference/troubleshooting.md`](reference/troubleshooting.md) as each locale clears review (keeping the explicit note that `ar`/`he` RTL desktop GUI support is unlikely / unscheduled).

### R8.13 Older-macOS Metal device-lost hardening & Windows console suppression
Owner: open · Status: in progress · Was: platform & GPU compatibility audit 2026-11-24 · Depends on: none · Gate: post-Beta · Est. note: 4–8 h

- [x] Added `CREATE_NO_WINDOW` (`crates/platform/src/process.rs`) and documented older-macOS Metal / CPU-rasterizer fallback in [`troubleshooting.md#gpu-graphics`](reference/troubleshooting.md#gpu-graphics).
- [ ] Harden `apps/curvebook/src/gui_window.rs` to handle `wgpu` surface/device-lost or shader-compilation failures gracefully (clear native error dialog / structured stderr diagnostic instead of an opaque panic) and track the upstream `wgpu-hal` Metal shader-version fix when updating `wgpu`. *(`gui_window.rs` init/surface/device-lost diagnostics and native error dialogs landed locally; upstream `wgpu-hal` Metal shader-version update tracking remains open.)*

---

## R9 Post-parity scientific quality-of-life suite

Deferred items are unscheduled; existing persisted-file compatibility remains supported ([`qol-enhancements.md`](planning/qol-enhancements.md)).

### R9.10 Zero-install client-side WASM viewer & virtual workspace folder (`QOL-10`)
Owner: maintainers · Status: deferred · Was: QOL-10, M8+ · Scheduling: no milestone assigned

- Existing `VirtualWorkspaceFolder` code remains supported; further collaborator-viewer and folder-workspace QoL work is deferred.

### R9.11 Unpacked Git-friendly `.curvebook.d/` directory & checkpoints (`QOL-11`)
Owner: maintainers · Status: deferred · Was: QOL-11, M8+ · Scheduling: no milestone assigned

- Existing `.curvebook.d/` I/O and `NamedCheckpoint` commands remain supported; further QoL development is deferred.

### R9.13 Instant statistical, distribution & Bayesian calculators (`QOL-13`)
Owner: open · Status: deferred · Was: QOL-13, M8+ · Scheduling: no milestone assigned ([reference specification](planning/calculators-and-quick-stats.md))

- Deferred reference scope: stateless `calc.*` commands, scalar formula builtins, and a Calculators drawer.

### R9.15 Bench molarity, stock dilution & radiochemical decay calculators (`QOL-15`)
Owner: open · Status: deferred · Was: QOL-15, M8+ · Scheduling: no milestone assigned ([reference specification](planning/calculators-and-quick-stats.md))

- Deferred reference scope: molarity, stock/serial dilution, moles/grams, and radiochemical calculators.

---

## R10 Human-operable GUI & 50-workflow human completion

Automated five-stage completion of all `65` reference workflows (`WF-01`–`WF-65`), desktop import/export gaps (`R10.29`–`R10.30`), `.pzfx`/`.prism` naming (`R10.34`), Portfolio Gallery (`R10.35`), and Navigator rail icons (`R10.36`) are **done** and archived in [`history/roadmap/r10.md`](history/roadmap/r10.md).

**Binding rules:** (1) A step counts only when driven by raw pointer, wheel, keyboard, or IME events against drawn widgets (`EguiHumanPathDriver` or physical OS input; [pitfalls §6](pitfalls.md#6-treating-semantic-agent-harness-tests-as-proof-of-human-gui-parity)). (2) All wording, icons, and styling are original (`xtask {brands,cleanroom,assets}`); lowercase `".pzfx files"` and `".prism files"` are explicitly allowed and preferred (`AGENTS.md` §1).

### R10.13 Human-operable 50-workflow acceptance & manual QA
Owner: maintainers · Status: in progress · Was: Human-GUI audit 2026-11-15 · Est. note: 50–80 h · Depends on: [`R10.25`](history/roadmap/r10.md#r1025-automated-gui-completion-of-all-65-workflows-beta-gate), [`R14.1`](#r141-beta-release-pipeline), [`R11.10`](#r1110-live-browser-qa--supported-browser-matrix) · Gate: **1.0** (Beta relies on automated [`R10.25`](history/roadmap/r10.md#r1025-automated-gui-completion-of-all-65-workflows-beta-gate))

- [x] Automated `human_wf_*` suites (`65 / 65` five-stage complete), screenshot baselines (`cargo xtask screenshots --check`), and [`gui-workflows.md`](gui-workflows.md) §6.2 evidence table are complete and enforced by `cargo xtask status`.
- [ ] **Desktop (1.0):** Run and record a walkthrough of all 65 workflows (`WF-01`–`WF-65`, plus kinetic microplate/AUC UI spot-checks from [`R17.7`](history/roadmap/r17.md#r177-multi-block-tabular-import-standalone-auc-analysis-and-composable-microplate-mapping)) on the macOS desktop app, installed from a signed release candidate build ([`R14.1`](#r141-beta-release-pipeline) pipeline), using either an agent with "computer use" mouse and screen (driving the real rendered window strictly from screen pixels, OS mouse movement, clicks, drags, wheel scroll, and keyboard input — never `--control`, MCP, `--event-script`, or `dispatch_gui_action`) or a manual human. Include the real native open/save/export dialogs and the system clipboard, which the human-path harness stubs. File defects against new `R10` items; **1.0** ships only when every workflow completes. *(Moved from **Beta** 2026-11-20.)*
- [ ] **Browser (1.0):** Run and record the same walkthrough in one browser under the same rules, delivered by [`R11.10`](#r1110-live-browser-qa--supported-browser-matrix), filing defects against `R11` items.

### R10.24 Windows & Linux desktop GUI smoke QA (1.0 gate)
Owner: maintainers · Status: in progress · Was: Beta GUI scope review 2026-11-18 · Est. note: 12–24 h, plus Windows/Linux desktops · Depends on: [`R14.1`](#r141-beta-release-pipeline), [`R10.23`](history/roadmap/r10.md#r1023-close-the-remaining-desktop-gui-gaps-beta-gate) (done), [`R15.5`](history/roadmap/r15.md#r155-default-desktop-egui-cutover-screenshot-refresh-and-beta-readiness) (done) · Gate: **1.0**

- [x] Defined the smoke walkthrough in [`desktop-smoke-qa.md`](release/desktop-smoke-qa.md) and scoped FreeBSD as Tier 2 CLI-only (2026-11-18).
- [ ] **Windows 11:** run it on `x64` from the unsigned `.msi` (record the SmartScreen prompt for the Beta release notes), at 100% and 200% display scaling; on `arm64` where hardware is available.
- [ ] **Linux `x86_64`:** run it from the AppImage (the only Linux format in the [`R14.1`](#r141-beta-release-pipeline) Beta download set), in a Wayland session and an X11 session, using the XDG file-chooser portal, at 1x and 2x scaling.
- [ ] Use computer-use (mouse and screen) or a manual human on real or virtual desktops. GitHub-hosted runners have no interactive session, and semantic-harness runs do not count ([pitfalls §6](pitfalls.md#6-treating-semantic-agent-harness-tests-as-proof-of-human-gui-parity)). Record OS version, build, steps, and results in [`STATUS.md`](STATUS.md), and file defects against `R10.23` or new `R10` items. **1.0** ships only when the walkthrough completes on every listed system.

### R10.31 Desktop graph, image, gallery & linked-region UI parity fixes
Owner: open · Status: in progress · Was: GUI & layout spec audit 2026-11-24 · Depends on: none · Gate: post-Beta · Est. note: 8–14 h

- [x] Fixed `build_legend_items` dataset-title fallback (`column_grouped/`) and targeted linked-region **Unlink** (`linked_regions.rs`) with regression tests.
- [ ] Expose a `Bar width (pt)` numeric spinner bound to `SeriesSpec::bar_width_override_pt` in `crates/ui-egui/src/egui_shell/graph_modals/format_graph/series.rs`.
- [ ] Expose `(X, Y)` position inputs and a `pt`/`mm`/`cm`/`in` unit selector in `render_format_image_geometry` (`crates/ui-egui/src/egui_shell/graph_modals/r13_modals.rs`).
- [ ] Add `Export Selected` and `Place Selected onto New Layout` batch actions to `PortfolioGallery` (`crates/ui-egui/src/egui_shell/portfolio_gallery.rs`) and a `Target Scope` toggle (`Active sheet` vs `Selected gallery sheets`) to `ActiveDialog::ExportFigure` (`crates/ui-egui/src/egui_shell/app_modals.rs`).

### R10.32 Desktop workbench authoring, Welcome Hub, Paste Transpose & sheet-lifecycle ergonomics
Owner: open · Status: in progress · Was: desktop workbench spec audit 2026-11-24 · Depends on: none · Gate: post-Beta · Est. note: 16–28 h

- [x] Surfaced inline `welcome.sample.error` and disabled unavailable sample modes cleanly in `welcome_hub.rs`.
- [x] **Floating Notes authoring (Slice 1):** added typed engine commands (`sheet.addFloatingNote`, `sheet.updateFloatingNote`, `sheet.deleteFloatingNote`), desktop toolbar/menu/card authoring controls to create, edit, recolor, drag, minimize, and delete `FloatingNote` cards (`center_stage.rs`, `top_chrome.rs`, `menu_bar.rs`), and optional instructional notes on tutorial sample fixtures (`workspace_nav.rs`, `welcome_hub.rs`).
- [x] **Direct Paste Transpose & interactive row sorting (Slice 1):** added one-click `Edit › Paste Transpose` (`table.pasteTranspose`), `TableArchetype::permute_rows` / `DataSheet::permute_rows`, `table.sortRows`, and interactive data-grid row-sorting column-header/menu/dialog controls (`crates/ui-egui/src/egui_shell/data_grid/`).
- [x] **Clone a Graph replicate override, Slice 1 catalog/i18n sync & `EguiHumanPathDriver` coverage:** added the `override_replicates` subcolumn-format control to `Clone a Graph` (`welcome_hub.rs`), regenerated `docs/commands.md` and `crates/mcp/src/ui_catalog.rs`, added all new Slice 1 UI/command strings across all `16` locales, and added `EguiHumanPathDriver` regression tests covering Floating Notes authoring (`r15_3_stage_canvas.rs`), Welcome Hub sample notes & `override_replicates` (`r15_3_welcome_nav.rs`), Paste Transpose (`r15_4_grids_and_ingestion.rs`), and interactive row sorting (`r15_2_column_grid.rs`).
- [ ] **Sheet-lifecycle ergonomics (Slice 2):** add cascading auto-title rename propagation in `crates/engine/src/cmd/sheet.rs` (`run_sheet_rename` updates linked auto-titled `ResultSheet` and `GraphSheet` titles while preserving user-edited `custom_title`s), and replace hard `Conflict` rejection when deleting a `GraphSheet` embedded on a `LayoutSheet` with an unassigned placeholder frame on the layout.
- [ ] **Welcome Hub completions (Slice 3):** add Recently Accessed Files and Recently Accessed Folders MRU lists to `Open a File`; add cross-project / recent / saved-example source tabs to `Clone a Graph` (`crates/ui-egui/src/egui_shell/welcome_hub.rs`); and provide curated styled example portfolio projects alongside the starter templates.
- [ ] Add `EguiHumanPathDriver` regression coverage, `crates/mcp/src/ui_catalog.rs` updates, and `16`-locale catalog entries for remaining Slice 2–3 additions.

### R10.33 Advanced Import/Paste Special live-link persistence & Export Figure options
Owner: open · Status: in progress · Was: import/export spec audit 2026-11-24 · Depends on: [`R10.29`](history/roadmap/r10.md#r1029-desktop-import-and-export-gaps-beta-gate) (done) · Gate: post-Beta · Est. note: 14–22 h

- [ ] Persist `missing_sentinels`, `filter_pattern`, `ByRows` (`values_per_row` + `wrap_after_rows` for 96/384-well plate reshapes), and `ByColumns` stacking in `ImportFilterSpec` / `ImportPlacementSpec` (`crates/doc/src/links.rs`, `crates/transform/src/import.rs`) so block/plate reshapes work on live-linked imports as well as static pastes.
- [ ] Add `skipRowsUntil`, `skipRowsAfter`, `skipEveryRowWhere` (row omission vs exclusion), `decimateCols`, `renameTableMode` (`imported_filename` / `fromSourceRow1Based`), `<Info>...</Info>` / `<Notes>...</Notes>` tag and explicit row-range extraction in `ImportWizard` (`crates/ui-egui/src/egui_shell/import_wizard.rs`), and optional OS file-watcher refresh (`autoUpdateOnChange`) for `LinkSource::ExternalDelimitedFile`.
- [ ] Complete `Export Figure` options (`crates/ui-egui/src/egui_shell/app_modals.rs`, `crates/render/`): true alpha-channel transparent canvas background export, `Grayscale` and `1-bit Monochrome` color spaces, `JPG` and `BMP` raster output options, `Save as default`, and `Open folder after export`. *(True alpha-channel `transparent_bg` export for SVG/PNG/TIFF landed locally; `Grayscale` / `1-bit Monochrome`, `JPG` / `BMP`, `Save as default`, and `Open folder after export` remain open.)*

---

## R11 Usable browser app

**Status & architecture (deferred to `1.0`; not a Beta requirement):** Following desktop `egui` cutover (`R15.5`, `0bce30e`), the legacy inline-JS/native web bridge (`chrome_bridge.rs`, `text_bridge.rs`, `host_adapters.rs`, and legacy `HumanPathDriver`) was removed. `apps/curvebook-web` is currently a minimal strict-CSP, zero-egress `wasm32` placeholder pending [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate) (`eframe::WebRunner`). `R11.1`–`R11.10` retain the browser capability requirements that `R11.11` and subsequent `1.0` browser work must satisfy and prove in real browsers.

### R11.1 Real-browser test harness & web CI gates
Owner: maintainers · Status: in progress · Was: Browser-app audit 2026-11-17 · Est. note: 24–40 h

- [x] Paused automatic WASM CI (keeping `cargo xtask ci wasm` and `cargo xtask web-bundle` on demand), added `wasm32` Clippy + `trunk build --cargo-profile web-release`, and implemented `verify_web_bundle_zip_bytes` (`xtask/src/web_fonts.rs`).
- [ ] **Decision:** choose the in-browser test runner. Recommended: the `wasm-bindgen-test` dev-dependency run headless through an externally installed `wasm-bindgen-cli` test runner with chromedriver/geckodriver in CI only. Rust WebDriver client crates are excluded because they are HTTP clients (Zero Telemetry). Confirm with `cargo xtask licenses` and `cargo xtask telemetry`.
- [ ] Add a browser smoke suite (boot, first frame, click, type, open project bytes, export) and run it in CI when automatic WASM verification resumes.

### R11.2 Rust-owned browser host glue & direct frame upload
Owner: open · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 40–64 h · Depends on: `R11.1`, [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate)

- [ ] Keep DOM listeners, file pickers, multi-tab locks, and autosave scheduling in Rust (`web-sys`) modules under `apps/curvebook-web/src/` alongside `eframe::WebRunner`, keeping `apps/curvebook-web/index.html` a minimal strict-CSP shell.
- [ ] Avoid base64-in-JSON frame transport (use direct WebGPU/WebGL canvas presentation via `eframe::WebRunner` or direct wasm memory upload).
- [ ] Cover host glue with `R11.1` browser tests while keeping strict CSP, zero egress, and `cargo xtask telemetry` passing.

### R11.3 Browser text measurement drives layout
Owner: open · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 16–28 h · Depends on: [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate)

- [ ] Ensure browser-side font loading and text measurement in the `eframe::WebRunner` build match rendered hit boxes without network requests or unused Trunk `fonts/` copy hooks.
- [ ] Browser test: hit-box centres match the accessibility tree and drawn text is not clipped for long and CJK labels at `1.0x` and `2.0x`.

### R11.4 Text entry, IME, focus & pointer input
Owner: open · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 24–40 h · Depends on: [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate)

- [ ] Verify composition (IME), soft-keyboard, native cut/copy/paste, pointer capture for drags leaving the canvas, focus-exit (`Tab`/`Esc` outside modals), and touch inertia in the `WebRunner` browser host.
- [ ] Browser tests for IME commit in a grid cell (`ja`, `zh-CN`, `ko`), drag outside the canvas, and focus exit.

### R11.5 Document lifecycle: save-in-place, folders & recovery
Owner: open · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 20–32 h · Depends on: [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate)

- [ ] Save back to the retained file handle where the File System Access API allows it (download fallback unchanged), and mark the document clean after a successful save or export.
- [ ] Write opened `.curvebook.d/` folders back through `SyncDirectory`.
- [ ] Key autosave per document, prompt before restoring, and clear the snapshot after a successful save.
- [ ] Supply real timestamps on `wasm32` through the caller-supplied timestamp path (no `chrono`, no hand-rolled calendar math).

### R11.6 Responsiveness: off-main-thread compute & frame cadence
Owner: maintainers · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 32–56 h · Depends on: [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate)

- [ ] **Decision:** run the engine session in a same-origin Web Worker (CSP `'self'`) or make long commands chunked and cancellable on the main thread.
- [ ] Show a busy state with cancel for long analyses and fits; collapse duplicate resize renders; schedule frames cleanly with `requestAnimationFrame`.
- [ ] Measure the `R10.22` budgets in a browser on the `1M`-cell grid and a nonlinear-fit workflow.

### R11.7 Fonts, export & clipboard fidelity on web
Owner: open · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 12–20 h · Depends on: `R11.3`

- [ ] Let the user supply a TrueType/OpenType font file for PDF/SVG embedding (`SystemFontSet::from_sfnt_bytes`, `embedded_truetype_font` in `crates/pdf/src/writer.rs`).
- [ ] **Decision:** offer EMF as a web download, or document it as desktop-only.
- [ ] Support image paste from the clipboard, or document it as unsupported on web.

### R11.8 Browser accessibility
Owner: open · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 16–28 h · Depends on: [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate), `R11.4`

- [ ] Ensure the browser accessibility tree exposes actionable, incrementally updated controls routed through the same input path as pointer and keyboard events.
- [ ] Record a screen-reader walkthrough (VoiceOver with Safari, NVDA with Firefox); feeds [`R10.20`](history/roadmap/r10.md#r1020-keyboard--screen-reader-accessibility).

### R11.9 50-workflow human-path parity on the web host
Owner: open · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 30–48 h · Depends on: `R11.3`, `R11.4`, [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate)

- [ ] Run `WF-01`–`WF-65` human-path parity against the `WebRunner` web host (sharing `EguiHumanPathDriver` / browser test driver) plus a five-workflow subset in a real browser under `R11.1`.

### R11.10 Live browser QA & supported-browser matrix
Owner: maintainers · Status: todo · Was: Browser-app audit 2026-11-17 · Est. note: 16–24 h · Depends on: `R11.1`–`R11.9`, [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate) · Gate: **1.0**

- [ ] Record walkthroughs using an agent with "computer use" mouse and screen or a manual human on current Chrome or Edge, Firefox, Safari, and iPad Safari, filing defects against `R11` items.
- [ ] Confirm or correct the minimum browser versions stated in [`packaging/web/README.md`](../packaging/web/README.md).
- [ ] Use these walkthroughs as the browser part of [`R10.13`](#r1013-human-operable-50-workflow-acceptance--manual-qa) QA and the live browser QA of [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening) (required for the **1.0** full release, not **Beta**).

### R11.11 Migrate browser host to eframe WebRunner (1.0 gate)
Owner: maintainers · Status: todo · Was: Post-Beta browser overhaul boundary 2026-11-19 · Est. note: 36–60 h · Depends on: [`R15.5`](history/roadmap/r15.md#r155-default-desktop-egui-cutover-screenshot-refresh-and-beta-readiness) (done), `R11.1` · Gate: **1.0**

- [ ] Rebuild `apps/curvebook-web` onto `eframe::WebRunner` sharing [`R15`](history/roadmap/r15.md#r15-modern-egui-desktop-gui-overhaul-beta-gate)'s `egui` views, solving zero-`fetch(` staged first-paint font loading under CurveBook's strict shipping CSP (`AGENTS.md` §1) and keeping the compressed bundle within `MAX_WEB_RELEASE_ZIP_BYTES` (`6,243,591` B in `xtask/src/web_fonts.rs`).
- [ ] Use tagged Serde messages for the surviving scalar metadata browser-bridge events/commands (`R11-1` maintainability follow-up: keep frame pixels, file bytes, and legacy compatibility decoding behind adapters until characterized).
- [ ] Reconcile or fold superseded subtasks in `R11.2`–`R11.9` once `WebRunner` runs under `R11.1`, and verify `R11` IME, browser accessibility, save/recovery, clipboard, and `R11.10` live browser acceptance.

---

## R12 User documentation

**Scope & rules:** `cargo run -p xtask --features docs-site -- docs-site` renders hand-written Markdown under `docs/reference/` and typed workspace registries into a deterministic, script-free static HTML manual staged in CI (gitignored `target/docs-manual/` -> `_site/manual/`, never committed under `docs/manual/`; see [`R12.10`](history/roadmap/r12.md#r1210-generate-manual-html-purely-in-ci) and [`R21.4`](history/roadmap/r21.md#r214-reduce-documentation-footprint-audit-bloat-and-generated-files)). Desktop GUI paths may be documented once covered by a passing five-stage `WorkflowRun` or `R10.30` import/export test (cited in the page source; [pitfalls §6](pitfalls.md#6-treating-semantic-agent-harness-tests-as-proof-of-human-gui-parity)). User-facing documentation prefers lowercase `".pzfx files"` and `".prism files"` naming (`AGENTS.md` §1).

### R12.1 Documentation site on GitHub Pages
Owner: maintainers · Status: in progress · Was: Maintainer request 2026-11-18 · Est. note: 24–40 h · Gate: **Beta**

- [x] Built the deterministic static manual generator (`xtask/src/docs_site/`), flat readability template (`R12.11`), navigation/A–Z index/glossary, registry-generated pages, CI-only staged publication (`R12.9`–`R12.10`, `R21.4`), `--check` policy/readability/CSP gates, and landing-page entry points (2026-11-18..2026-11-26).
- [ ] Push the pending landing-page Alpha wording and the `README.md` website/release links, then record the public-result checks in [`github-pages.md`](release/github-pages.md). Pages is published at <https://curvebook.org/> via [`.github/workflows/pages.yml`](../.github/workflows/pages.yml) (`R12.9`). *(from R0.14)*
- [ ] Verify the published site with the checks in [`github-pages.md`](release/github-pages.md#preview-and-checks) (desktop and narrow layouts, keyboard focus, contrast, links) and record the hosted verification. Run it as an automated check: a script fetches every URL in the published `sitemap.xml` and follows every internal link, and the layout, focus, and contrast checks run as a script or an agent driving a browser. No manual step (decided 2026-11-24).

### R12.3 Functionality and methods reference
Owner: open · Status: in progress · Was: split from `R12.1` 2026-11-18 · Depends on: `R12.1`, [`R0.30`](history/roadmap/r0.md#r030-api-documentation-for-numerical-and-model-crates) · Gate: none

- [x] Authored neutral clean-room reference guides under `docs/reference/` (`concepts.md`, `tables.md`, `analyses.md`, `fitting.md`, `graphs.md`, `qol.md`) verified by `docs-site --check`, `brands`, and `cleanroom` (2026-11-18..2026-11-23).
- [ ] **One wording for reported values:** describe each reported value with the wording of its [`R0.30`](history/roadmap/r0.md#r030-api-documentation-for-numerical-and-model-crates) field doc (copy it, or generate it if the site generator can read it). Where the two disagree, fix the field doc first so the code docs and the published reference give the same meaning. *(from R0.30)*

### R12.2 Example workflow guides
Owner: open · Status: todo · Was: Maintainer request 2026-11-18; pilot scoped 2026-11-24 · Est. note: 35–55 h for the pilot below · Depends on: `R12.1`, `R12.3`, [`R12.4`](history/roadmap/r12.md#r124-automation-reference-mcp-cli-control-protocol-and-batch), **Beta** release; GUI steps cite [`R10.25`](history/roadmap/r10.md#r1025-automated-gui-completion-of-all-65-workflows-beta-gate) / [`R10.30`](history/roadmap/r10.md#r1030-human-path-tests-for-import-and-export-beta-gate) · Gate: post-Beta

- [ ] Write step-by-step guides for common scientific workflows (for example two-group comparison, one-way ANOVA with multiple comparisons, dose-response curve fitting, survival analysis, contingency tables, and a publication figure with Page Layout export), each with an MCP or CLI path and a GUI path wherever a passing human-path test covers it. Organise guides by user task and name them after it; regroup the reference workflows instead of writing one guide per `WF-NN`, and leave out steps that only exist to test the software (guard and predicate checks).
- [ ] **Pilot (scoped 2026-11-24):** five guides, in this order:
  1. *Convert `.prism` and `.pzfx` files to CSV* — `curvebook-cli info` and `convert`, `--sheet`, every table, TSV, decimal commas, and converting to a `.curvebook` project; extends `apps/curvebook-cli/tests/cli_suite/r12_8_pzfx_landing_page.rs`. Describe only what the importers read and show how skipped parts are reported. Cite the passing [`R10.30`](history/roadmap/r10.md#r1030-human-path-tests-for-import-and-export-beta-gate) human-path tests (`crates/ui-egui/tests/human_wf_suite/human_wf_import_export.rs`) for every GUI import/export step.
  2. *One-way ANOVA with Tukey, and exporting the results* (`WF-10`, `WF-24`, `WF-50`) — diagnostics, Tukey or Dunnett, Welch ANOVA, brackets and compact letter display, Kruskal–Wallis; export results as CSV (one tab or all tabs, delimiter and decimal separator), JSON, and a methods report, and the figure as SVG/PDF. Cite the passing [`R10.30`](history/roadmap/r10.md#r1030-human-path-tests-for-import-and-export-beta-gate) human-path tests (`human_wf_import_export.rs`) for every GUI export step.
  3. *Fit a dose-response curve and find the EC50* (`WF-17`, `WF-36`, `WF-51`, `WF-52`) — log X, 4PL (and when 5PL is justified), EC50 with its interval, interpolating unknowns, outlier badge, unstable parameters.
  4. *Kaplan–Meier survival with log-rank and number at risk* (survival part of `WF-21`, `WF-47`, `WF-53`) — median survival, log-rank and pairwise tests, hazard ratio, number-at-risk table, caveats.
  5. *Make a publication figure* (`WF-24`, `WF-25`, `WF-27`, `WF-28`) — graph type and error-bar choice, colour-vision-safe palettes, a two-panel journal layout built from guides 2 and 3, and export at a set size and resolution.
- [ ] **Pilot groundwork:** one page template (goal, data, steps, reading the result with links to the `R12.3` methods reference, export, next steps); original datasets from seeded generator scripts in the repository; and a gate that checks every guide step against a step in a passing test. Decide whether guides live in a versioned **Guides** section of the manual (published with the next release snapshot) or as top-level pages.
- [ ] Use original or properly licensed sample data with `ASSETS.md` / `ATTRIBUTION.md` rows; do not reproduce tutorial examples or datasets from other products (clean-room policy in [`AGENTS.md`](../AGENTS.md)).
- [ ] Keep the guides executable: back each guide's MCP steps with a test (for example a `curvebook.batch.v1` script or a `command_batch` sequence) that runs in CI, so a guide fails CI when the product changes under it.
- [ ] Publish the guides on GitHub Pages next to the reference and link them from the README and the beginner's guide.

### R12.7 Application user guide (desktop and browser)
Owner: open · Status: todo · Was: Maintainer scope decision 2026-11-18 · Est. note: 32–56 h · Depends on: `R12.1`, `R12.3`, [`R10.25`](history/roadmap/r10.md#r1025-automated-gui-completion-of-all-65-workflows-beta-gate), [`R10.30`](history/roadmap/r10.md#r1030-human-path-tests-for-import-and-export-beta-gate); browser part on `R11.10` · Gate: none (browser part waits for **1.0**)

- [ ] Add a **Using the app** section: workspace layout, navigator and sheets, menus and dialogs (from `UI_COMMANDS`), keyboard shortcuts (generated from the same registry), accessibility and screen-reader support, and how each `R12.3` capability is reached in the interface.
- [ ] Document a GUI path only when a passing human-path test or a recorded computer-use or manual walkthrough covers it, and cite that evidence in the page source ([pitfalls §6](pitfalls.md#6-treating-semantic-agent-harness-tests-as-proof-of-human-gui-parity)). Agent-harness tests do not count.
- [ ] Take screenshots from the clean-room screenshot generator (`cargo xtask screenshots`), with an `ASSETS.md` row for each (`cargo xtask assets`).
- [ ] Add the browser app's differences (file access, saving, fonts, supported browsers) once `R11.10` passes.

### R12.8 Search discoverability for the Pages site
Owner: open · Status: in progress · Was: Maintainer request 2026-11-23 · Est. note: 4–8 h · Depends on: `R12.1` · Gate: none

- [x] Added CI-generated `sitemap.xml`, canonical `/manual/latest/` URLs, Open Graph/Microdata, LLM indexes, and [`open-pzfx-files.html`](open-pzfx-files.html) (verified by `apps/curvebook-cli/tests/cli_suite/r12_8_pzfx_landing_page.rs`; see [`github-pages.md`](release/github-pages.md#search--ai-discoverability-metadata-r128-r1210-r214)).
- [ ] **Search Console:** the maintainers verify the site (an HTML verification file or meta tag; no DNS access is needed) and submit the sitemap. *(Maintainer action.)*
- [ ] Follow-ups need their own decisions: add task-oriented analysis pages once `R12.2` starts (`R12.9` site-only publishing and `R12.10` CI-only manual HTML are archived in [`history/roadmap/r12.md`](history/roadmap/r12.md)). Further options, with their status and policy constraints, are catalogued in [`seo-actions.md`](release/seo-actions.md).

---

## R14 Releases & distribution

`R14.1` consolidates all hosted Beta release work (`R0.6`, `R0.14`, `R0.24`, `R0.26`, `R0.28`, `R8.10`) plus hosted `ci.yml` verification for locally-landed waves (`R0.32`–`R0.34`, `R14.6`, `R16.19`, `R17.1`, `R17.6`, `R17.7`, `R20.1`, `R22`).

### R14.1 Beta release pipeline
Owner: maintainers · Status: in progress · Was: merged 2026-11-18 from `R0.28`, `R0.26`, `R8.10`, and hosted-run steps of `R0.6`, `R0.14`, `R0.24` · Est. note: 18–34 h · Depends on: Apple credentials (configured 2026-11-18) · Gate: **Beta**

- [x] **Download set, macOS signing, release-scan, NIST cache & maintainer docs:** Reduced Beta asset set (`--expect-beta-set`, Windows `x86` dropped, consistent filenames), per-job `release-scan`, fail-closed Developer ID signing/notarization in `release.yml`, cached NIST StRD corpus fetch in `ci.yml`, and updated [`releasing.md`](release/releasing.md) / [`get-started.md#verifying-downloads`](reference/get-started.md#verifying-downloads) (2026-11-18..2026-11-19).
- [ ] **Fix the Windows `.msi` scan.** Confirm the local `6cb67c0` fix (`absolute_tool_path` in `xtask/src/release_scan.rs` fixing `msiexec /a` exit `1619` on relative paths) passes on a hosted Windows release dry run (`x64` and `arm64`).
- [ ] **Hosted CI (`main`):** Confirm `ci.yml` passes on `main` on GitHub Actions (covering `cargo xtask workflows` and all locally-landed waves including archived `R0.32`–`R0.34`, `R14.6`, `R16.19`, `R17.1`/`R17.6`/`R17.7`, `R20.1`, and `R22`), and record the run URL.
- [ ] **Rehearsal release:** Push the next prerelease tag (currently planned as `v0.1.0-alpha.2`; the tag must equal the workspace version) through the native `release.yml` jobs with the Apple credentials active. In that one run confirm that:
  - every build job and scan step passes, and hosted `cargo deny`/`cargo about` run on every runner;
  - the asset list equals the declared set, with no `-windows-x86` and no web `.zip`;
  - the app, the CLI inside it, and the `.dmg` carry Developer ID signatures, notarization is accepted, and the tickets are stapled;
  - every asset passes `gh attestation verify --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml --source-ref refs/tags/v<version>` and `SHA256SUMS.txt`.
  Record the run URL, the version, and the asset list.
- [ ] **Clean-machine checks on the rehearsal assets (automated; changed 2026-11-24 from a maintainer check on a clean Mac):**
  - On a fresh GitHub-hosted macOS runner (a clean virtual machine per job), download the `.dmg` so it carries the quarantine attribute, then check that it passes Gatekeeper without a bypass (`spctl --assess` on the `.dmg` and on the mounted app, `stapler validate`) and that the app and the bundled `curvebook-cli --version` start.
  - On a Windows runner, installing the `x64` MSI over an Alpha `x86` MSI (same `UpgradeCode`, per-user and per-machine) removes the 32-bit copy and leaves one Start-menu entry. If it doesn't, document a manual uninstall.
  Record the results.

### R14.2 Release SBOM and CI runner hardening
Owner: maintainers · Status: in progress (Part A SBOM landed; Part B decision & hosted CI open) · Was: infrastructure drift audit 2026-11-21 · Depends on: [`R14.1`](#r141-beta-release-pipeline) · Gate: none · Est. note: 4–8 h

- [x] **Part A — CycloneDX SBOM:** Added deterministic CI-only `cargo-cyclonedx` + [`packaging/sbom.sh`](../packaging/sbom.sh) generating `curvebook-<version>.cdx.json`, `actions/attest-sbom` verification in `release.yml`, required `AssetKind::Sbom` checks in `xtask/src/release_scan.rs`, and doc updates (2026-11-25).
- [ ] **Decision:** add a runner-hardening step (network egress allowlist) to `ci.yml` and `release.yml`, or record why not. Adopt only a mode that sends no build or network data to a third-party service. Pin any new action by commit SHA and keep `cargo xtask telemetry` and `cargo xtask workflows` passing.
- [ ] Verify: hosted CI and one release dry run pass.

### R14.3 Retry transient release verification failures
Owner: maintainers · Status: in progress · Was: hosted CI and release review 2026-11-21 · Depends on: [`R14.1`](#r141-beta-release-pipeline) · Gate: none · Est. note: 1–3 h

- [x] Added bounded retries (`retry_notarization_check` in `packaging/macos/verify.sh`, `6cb67c0`) for network-backed `stapler validate`/`spctl --assess` checks; confirmed no FreeBSD ZFS ARC OOM incident on record.
- [ ] Verify: a hosted release dry run passes, and a forced failure still fails after the last attempt.

### R14.4 Homebrew tap for macOS (post-Beta)
Owner: maintainers · Status: todo · Was: Maintainer request 2026-11-24 · Depends on: [`R14.1`](#r141-beta-release-pipeline) · Gate: post-Beta · Est. note: 1–2 h

- [ ] Create public repository `HauserGroup/homebrew-tap` with `Casks/curvebook.rb` pointing at the signed, notarized `curvebook-<version>-macos-universal.dmg` release asset and its `SHA256SUMS.txt` checksum, installing `CurveBook.app`, symlinking `CurveBook.app/Contents/MacOS/curvebook-cli` onto `PATH`, and declaring `zap trash` paths matching [`privacy.html`](privacy.html).
- [ ] Document the per-release cask bump (manual or tap-side workflow) in [`releasing.md`](release/releasing.md), keeping `curvebook`'s `release.yml` permissions and egress unchanged.
- [ ] Add `brew install --cask hausergroup/tap/curvebook` to [`get-started.md`](reference/get-started.md) and link it from [`README.md`](../README.md) once the tap is live.
- [ ] Verify on macOS: `brew install --cask hausergroup/tap/curvebook` passes Gatekeeper, `CurveBook.app` launches, `curvebook-cli --version` runs from `PATH`, and `brew uninstall --cask curvebook` removes the app and CLI symlink cleanly. *(Follow-up after `1.0` and Homebrew notability threshold: submit the same cask to official `Homebrew/homebrew-cask`.)*

### R14.5 MCP Bundle (`.mcpb`) and MCP Registry listing (post-Beta)
Owner: maintainers · Status: todo · Was: Maintainer request 2026-11-24 · Depends on: [`R14.1`](#r141-beta-release-pipeline) · Gate: post-Beta · Est. note: 4–6 h

- [ ] Add an `.mcpb` packaging template (`manifest.json` with `server.type: "binary"`, `args: ["mcp", "--workdir", "${user_config.workdir}"]`, `user_config` directory picker, and attributed app icon) and build per-platform `.mcpb` archives in `.github/workflows/release.yml` reusing the signed macOS `curvebook-cli` and platform release binaries.
- [ ] Extend `cargo xtask release-scan` (`--expect-beta-set` / post-Beta asset set) and `SHA256SUMS.txt` / Sigstore attestations to unpack, verify, and attest the `.mcpb` ZIP archives.
- [ ] Add `server.json` with plain-language discovery metadata (without bloating individual MCP tool descriptions) and publish the listing to the official MCP Registry (and optionally Smithery / Glama).
- [ ] Document one-click `.mcpb` installation in [`mcp-for-beginners.md`](mcp-for-beginners.md) and verify end-to-end install and a sample analysis in a desktop MCP client on macOS, Windows, and Linux.

### R14.7 Software citation: `CITATION.cff`, Zenodo DOI & versioned methods paragraph (post-Beta)
Owner: maintainers · Status: in progress (`CITATION.cff`, versioned methods report, desktop Cite dialog, homepage popover & citation docs landed; Zenodo DOI activation pending Beta release) · Was: Maintainer request 2026-11-26 (website "easy to cite" claim review) · Depends on: [`R14.1`](#r141-beta-release-pipeline) (Beta release) · Gate: post-Beta · Est. note: 8–12 h (including the GUI Cite dialog), plus Zenodo account setup

Researchers need a stable reference to cite CurveBook in a methods section. Zenodo mints one DOI per GitHub release plus a concept DOI that always resolves to the latest version. Use the concept DOI wherever the text is fixed at build time, because a release's own version DOI does not exist until Zenodo archives that release.

- [x] Add a root `CITATION.cff` (title, authors from [`contributors/`](../contributors/), `MIT OR Apache-2.0`, repository URL, version, and release date) and a check that its `version` matches the workspace version, so Zenodo and GitHub's "Cite this repository" pick up correct metadata. *(Done 2026-11-27: single-source `CURVEBOOK_CITATION` in `crates/doc/src/citation.rs`, root [`CITATION.cff`](../CITATION.cff) in CFF 1.2.0 using collective author `"CurveBook Contributors"`, atomic `cargo xtask version set` sync and `version::check_citation_cff` in `ci::policy`, plus exact drift checking in `xtask/src/docs_site/verify.rs`.)*
- [ ] Enable the Zenodo GitHub integration for the repository (configured on the Zenodo and GitHub side; no change to `.github/workflows/*` permissions or egress, per `cargo xtask telemetry`), publish one release through it, and record the concept DOI in `CURVEBOOK_CITATION.concept_doi`, `CITATION.cff`, and the synced docs.
- [x] Include the CurveBook version and concept DOI in the methods paragraph (for example "CurveBook 1.4.0 (doi:10.5281/zenodo.NNNN)"). Pass the version in rather than reading it inside the formatter, so `methods_report_outputs_stay_byte_identical` stays stable across version bumps, and keep the change `wasm32`-clean. *(Done 2026-11-27 in `crates/engine/src/methods_report.rs`: added `SoftwareAttribution` + `generate_methods_report_for_software` formatting `CurveBook <version>` and `(doi:<concept_doi>)` once `concept_doi` is set, kept `methods_report_outputs_stay_byte_identical` on a fixed `"0.0.0-fixture"` attribution, and verified `wasm32-unknown-unknown` check + `r14_7_methods_report_version_and_optional_doi_across_formats` across plain, Markdown, and LaTeX.)*
- [x] Add a "How to cite CurveBook" section to the documentation site and [`README.md`](../README.md) with the concept DOI and a ready-to-paste reference. *(Done 2026-11-27 in [`docs/reference/get-started.md`](reference/get-started.md#how-to-cite), [`docs/faq.html`](faq.html#faq-cite), [`README.md`](../README.md), and [`docs/release/releasing.md`](release/releasing.md), enforced against `CURVEBOOK_CITATION` by `check_software_citation_sync`; the concept DOI slot populates once minted.)*
- [x] Add a **Cite CurveBook** button to the desktop GUI. Register a `UiCommandSpec` (for example `app.cite`, next to `app.about` in `crates/ui-egui/src/menus/ui_commands.rs`, following [`command-ids.md`](command-ids.md)) and regenerate `crates/mcp/src/ui_catalog.rs` (`cargo xtask ui-commands`). Reach it from the Help menu, the About dialog, and a "Copy citation" button beside "Copy methods paragraph" in the result `Methods` panel (`crates/ui-egui/src/egui_shell/trailing_studio/results_interpret.rs`). It opens a small dialog with the reference for the running version, BibTeX, copy-to-clipboard buttons, and a DOI link opened only on click through `open_external_url` (no automatic network access). Keep one source for the citation text, shared with `CITATION.cff` and the methods paragraph and checked for drift, add i18n keys for every locale, and cover the dialog with a human-path test on `EguiHumanPathDriver` ([pitfalls §6](pitfalls.md#6-treating-semantic-agent-harness-tests-as-proof-of-human-gui-parity)). *(Done 2026-11-27: `app.cite` registered & `crates/mcp/src/ui_catalog.rs` regenerated, `ActiveDialog::Cite` reachable from Help menu, About dialog, and dot-free `Cite` button in the result `Methods` studio with 14-locale non-ellipsizing adaptive layout, exact-match `validate_external_url`/`open_allowlisted_external_url`, `12` keys across all `16` locales, and `EguiHumanPathDriver` coverage in `feedback_github_issue.rs`.)*
- [x] Add a small "Cite CurveBook" control directly under the home-page `Download` button ([`index.html`](index.html) hero `action-row`) that opens a citation pop-up: a plain-text reference, BibTeX in a `<details>` block, and links to `CITATION.cff` and the "How to cite" section. Use the native HTML `popover` attribute (`popovertarget`), which needs no JavaScript, so the strict script-free page CSP enforced by `xtask/src/docs_site/verify.rs` stays unchanged. Instead of a copy button, use CSS `user-select: all` so one click selects the whole reference. Check keyboard opening, `Esc` and light dismiss, focus return, and screen-reader labelling, and that browsers without CSS anchor positioning still show the pop-up centred. *(Done 2026-11-27 in [`docs/index.html`](index.html) and [`docs/site.css`](site.css): verified in browser at `1280×900` and `390×844` with zero JS/CSP/console errors, keyboard `Enter` open, `Esc` focus return, light dismiss, `role="dialog"` + `aria-labelledby`, `user-select: all`, expanded BibTeX fitting inside viewport, and centered mobile fallback.)*
- [ ] Only after the DOI resolves: allow website copy saying CurveBook is easy to cite in a methods section. Do not publish that claim earlier. *(Enforced by `check_software_citation_sync` in `xtask/src/docs_site/verify.rs`, which fails CI if `"easy to cite"` or a placeholder `"10.5281/zenodo."` appears while `CURVEBOOK_CITATION.concept_doi.is_none()`.)*
- [ ] Verify: the DOI resolves to the latest release, GitHub shows "Cite this repository", and a generated methods paragraph in plain, Markdown, and LaTeX formats contains the running version and the DOI.

---

## R16 Core- and GUI-crate maintainability (no gate)

Completed maintainability items (`R16.2`–`R16.19`) are archived in [`history/roadmap/r16.md`](history/roadmap/r16.md). Deferred Tier 2 splits below are unscheduled; any future split preserves public re-exports, `D1` tolerances, and `//!` citations (`cargo xtask citations`).

### R16.1 Split the stats ANOVA module by method family
Owner: open · Status: deferred (Tier 1 `anova/`, `two_group/`, `multivariate/`, `categorical/` done; optional Tier 2 splits deferred 2026-11-27) · Was: Core-crate maintainability review 2026-11-19 · Scheduling: no milestone assigned · Gate: none

- [x] Split `crates/stats/src/{anova,two_group,multivariate,categorical}.rs` into method-family submodule directories with `//!` `D1` citations, retired `too_many_arguments`, and passing NIST StRD / `D1` suites (PR #43 & 2026-11-25).
- Deferred optional Tier 2 follow-up: apply the same pattern on demand (one module per PR) to remaining `curvebook-stats` modules (`survival.rs`, `regression.rs`, `power.rs`, `column.rs`; prioritize `survival.rs` and `regression.rs` if future `R19`/`R23` work touches them).

---

## R17 Post-Beta adoption, interoperability & real-world polish

Completed local items (`R17.1`, `R17.5`–`R17.7`) are archived in [`history/roadmap/r17.md`](history/roadmap/r17.md). Only external commercial-app, Windows/Linux hardware, and GUI-MCP-host QA checks remain open below.

### R17.2 Downstream vector and document export interoperability
Owner: open · Status: in progress · Was: post-Beta prioritization 2026-11-23 · Gate: post-Beta

- [x] Verified SVG/PDF/EMF/PNG/TIFF exports locally across Inkscape, Poppler, Ghostscript, `pdflatex`, and `sips`; fixed writer interop bugs and shipped `ui.clipboard.copyForSlides` (`407c855`, [`evidence/r17-2-export-interop.md`](evidence/r17-2-export-interop.md)).
- [ ] Verify exported figures and native vector/bitmap clipboard paste interactively in Adobe Illustrator, Microsoft Word, and Microsoft PowerPoint on macOS and Windows (applications not installed / Windows host unavailable in local verification environment).

### R17.3 Real-hardware idle power and large-project responsiveness pass
Owner: open · Status: in progress · Was: post-Beta prioritization 2026-11-23 · Gate: post-Beta

- [x] Eliminated continuous macOS idle repaint loops, fixed large-project frame/undo hot paths (`resolve_cached_scene`, `Arc<UndoEntryInner>`, O(1) `SingleCellDelta`), and recorded `--release` benchmarks (`057329e`, [`evidence/r17-3-idle-power-perf.md`](evidence/r17-3-idle-power-perf.md)).
- [ ] Measure physical hardware idle CPU/GPU power draw on Windows (Task Manager / WPR / Battery & Energy Usage) and Linux (`powertop` / laptop power meter) laptops.

### R17.4 Live-LLM MCP usability trials and ergonomics fixes
Owner: open · Status: in progress · Was: post-Beta prioritization 2026-11-23 · Gate: post-Beta

- [x] Ran scripted live stdio JSON-RPC workflows, compacted MCP responses/schemas (`doc_inspect`, `command_list`, `compact_json_schema`, remediation hints), refreshed the MCP docs against the shipped server (`82d9243`: `command_batch` step limit, schema `v9` `doc_inspect`, `.pzfx`/`.prism` + `append`, resource query controls, retired `ui_answer_dialog` / `7`-tool `gui` count, ChatGPT Work/Codex setup), and backed every change with regression suites (`a06addb`, [`evidence/r17-4-mcp-usability.md`](evidence/r17-4-mcp-usability.md)).
- [ ] Run interactive live-LLM trials inside external GUI MCP client hosts (Claude Desktop, Cursor, Codex GUI integrations).

---

## R23 Analytical, statistical, regression & formula breadth (optional before Beta; non-blocking)

**Goal (optional before Beta, non-blocking; decided 2026-11-26):** Close the remaining numerical, statistical, multivariable regression, nonlinear modeling, curve-calculus, simulation, and calculated-variable formula gaps surfaced by the 2026-11-26 reference audit in one coordinated analytical program (`L0`–`L5` first, sharing schema evolution, `D1` R `4.5.2` / `mpmath` oracle suites, `ALL_ANALYSIS_DESCRIPTORS` registration, and thin UI wiring). None of `R23.1`–`R23.6` blocks **Beta**; items may land opportunistically before Beta or immediately after. Completed items (`R23.1`) are archived in [`history/roadmap/r23.md`](history/roadmap/r23.md).

### R23.2 Complete lognormal hypothesis-testing suite & standalone normality/lognormality
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 2) · Depends on: none · Gate: optional before Beta (non-blocking) · Est. note: 16–26 h

- [ ] Fix the silent drop of the 4 already-computed lognormal normality tests (`lognormal_dagostino_pearson`, `lognormal_shapiro_wilk`, `lognormal_anderson_darling`, `lognormal_kolmogorov_smirnov`) in `build_normality_tab()` (`crates/stats/src/column.rs`), and expose standalone Normality & Lognormality output rows: `Number of impossible (<=0)`, Relative Likelihood summary (`Normal %`, `Lognormal %`, `LR`, `1/LR`, `Preferred distribution`), and dual Normal + Lognormal Q–Q tabs/plots.
- [ ] Implement the remaining lognormal hypothesis tests with `D1` R `4.5.2` oracles: **One-Sample Ratio *t*-test** (geometric mean vs. $\mu_0 > 0$), **Unpaired Lognormal Welch's *t*-test** (unequal geometric SD factors), **Lognormal One-Way ANOVA** (Ordinary, Brown–Forsythe & Welch, and Repeated-Measures / REML Mixed-Effects with geometric-mean ratio `B/A` post-hoc contrasts & CIs), and **Multiple Row Lognormal & Ratio Paired *t*-tests**.
- [ ] Register additive spec fields in `crates/doc/src/analysis.rs`, wire `curvebook-engine` (`methods_report.rs`, commands, schemas), expose controls in `crates/ui-egui`, and sync [`statistics-implementation.md`](implementation/statistics-implementation.md) and [`docs/reference/analyses.md`](reference/analyses.md).

### R23.3 Data reduction, curve calculus, unwired transforms & chained Monte Carlo simulation
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 4a) · Depends on: none · Gate: optional before Beta (non-blocking) · Est. note: 20–32 h

- [ ] Add standalone **`RowStatistics`** (`AnalysisKind` + `ALL_ANALYSIS_DESCRIPTORS`: per-row Totals, Means with SD/SEM/%CV/CI, Medians with Quartiles/Min–Max/Percentiles, and Geometric Means with GeoSD/CI, emitting an analyzable table + auto-graph) and standalone **`FractionOfTotal`** (row, column, grand, or all-three totals + exact/Wilson binomial proportion CIs, retiring dead `TransformKind::FractionOfTotal`).
- [ ] Wire `apply_curve_calculus_to_sheet` (`crates/transform/src/curve.rs`) end-to-end as first-class XY analyses for **1st Derivative**, **2nd Derivative**, **Cumulative Integral**, and **Savitzky–Golay Smoothing** (coordinating directly with [`R19.2`](history/roadmap/r19.md#r192-high-usage-statistical-analyses--graph-families) smoothing).
- [ ] Expose the 4 already-implemented `BaselineKind` variants (`ColumnsMean`, `Constant`, `RowRangeMean`, `FirstAndLastLinear`) through `TransformAnalysisSpec::RemoveBaseline` alongside paired every-other-column, OLS-with-X, Add/Multiply, and retain-unselected-columns options; add pharmacology linearizing presets (Eadie–Hofstee, Hanes–Woolf, Hill, Scatchard), `Interchange X and Y`, zero-concentration substitution, asymmetric 95% CI error-bar transform policy, and extended `Extract & Rearrange` options (`Complete Rows Only`, `Subject Variable`, `Custom Row Titles`, Contingency count variable).
- [ ] Implement **Chained Monte Carlo Simulation** (deterministic seeded loop over $N$ iterations driving downstream analyses, tabulating hooked constants, evaluating `"Hit"` criteria into `Hits`/`Not Hits`/`All` tabs, and wiring `FitFlagRules::exclude_flagged_from_summary`), plus **Simulate $2 \times 2$ Contingency** (4 sampling designs) and random-effects/outlier Column simulation; back all new routines with `D1` R/SciPy oracles and bounded iteration budgets.

### R23.4 RM 2-/3-way sphericity, ANOVA completeness, consensus K-means, PCA depth & survival power
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 4b) · Depends on: none · Gate: optional before Beta (non-blocking) · Est. note: 22–36 h

- [ ] Replace hardcoded `RmSphericityCorrection::None` in `crates/stats/src/anova/sheet/factorial.rs` with user-selectable Geisser–Greenhouse and Huynh–Feldt corrections on 2-way and 3-way RM ANOVA; add a 4-mode zero/negative REML variance-component policy, 2-way `"cells differing by only one factor"` + 3-tier family partitioning, preselected comparison pairs, 3-way all-8 RM factor combinations & `Consolidate Data`, Multifactor selective individual terms & `Swap direction`, and Spearman heteroscedasticity checks in 2-/3-way/Multifactor ANOVA.
- [ ] Add a **K-Means 17-index consensus optimal-$k$ engine** (`Calinski–Harabasz`, `Davies–Bouldin`, `Dunn`, `Gap`, `Hartigan`, `Ball–Hall`, `Krzanowski–Lai`, etc.) with `Cluster metrics` tab and Gap plot, Hartigan–Wong and MacQueen K-means algorithms, K-means ANOVA summary tab, hierarchical clustering direction (`Rows`/`Cols`/`Both`) + independent scaling + cut-height clustering, and PCA custom eigenvalue / fixed-$k$ / cumulative-% retention + extra diagnostic matrix tabs.
- [ ] Add **Survival Power Analysis** (`PowerTestFamily::LogRankSurvival`, Schoenfeld / Freedman log-rank sample-size, power, and detectable hazard-ratio solves + sensitivity grid) and write formal implementation specs in [`statistics-implementation.md`](implementation/statistics-implementation.md) and [`graphing-and-layouts.md`](implementation/graphing-and-layouts.md) for [`R19.2`](history/roadmap/r19.md#r192-high-usage-statistical-analyses--graph-families) Frequency Distribution/Histogram and Gardner–Altman Estimation Plots; verify all statistical outputs against `D1` R `4.5.2` oracles.

### R23.5 Multivariable GLM (§5.4) & Cox PH regression completeness
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 3a; [`curve-fitting-implementation.md`](implementation/curve-fitting-implementation.md) §5.4) · Depends on: none · Gate: optional before Beta (non-blocking) · Est. note: 24–38 h

- [ ] Implement [`curve-fitting-implementation.md`](implementation/curve-fitting-implementation.md) §5.4 on `GlmSpec` (`crates/doc/src/analysis.rs`, `crates/stats/src/regression.rs`): 2-way and 3-way interaction terms, polynomial powers ($X^2, X^3$) and $\sqrt{X}$ transforms, `include_intercept: bool`, nested GLM comparison (Extra-SS *F* / deviance LRT / $\Delta\text{AIC}_c$), linear observation weights, Poisson Pearson dispersion & quasi-Poisson / sandwich SE scaling, row-level leverage / Cook's $D$ / DFBETAs / residual plots, automatic out-of-sample row prediction (blank $Y$), text 2-level outcomes, configurable logistic classification cutoff, profile-likelihood vs. Wald CIs, SE of OR/IRR, parameter correlation heatmap, and Multiple Logistic diagnostic graphs.
- [ ] Extend Cox PH & Kaplan–Meier (`crates/stats/src/survival.rs`): 2-/3-way interactions and predictor transforms in Cox PH, covariate-profile predicted survival curves, nested Cox likelihood-ratio model comparison, Cox VIF & parameter correlation matrix, Cox–Snell and covariate residual plots, configurable event/censoring codes & `"Treat other values as"` (enabling reverse-KM median follow-up), `MultipleVariables` multi-status/multi-grouping KM, and ratio of median survival + CI; verify every routine against `D1` R `4.5.2` (`stats::lm`, `stats::glm`, `survival::coxph`, `confint`) oracles.

### R23.6 Nonlinear regression constraints, derived-CI transforms, equation catalog, DSL depth & theoretical plotting (§6.2)
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 3b; [`curve-fitting-implementation.md`](implementation/curve-fitting-implementation.md) §6.2) · Depends on: none · Gate: optional before Beta (non-blocking) · Est. note: 24–40 h

- [ ] Add **relational parameter-vs-parameter inequality constraints** ($\theta_a > \theta_b$, e.g. `KFast > KSlow`, `KdLo > KdHi`), **derived-scalar SE/CI propagation** (extending `DerivedScalar` with delta-method / monotonic-endpoint CIs) and **`Y[expr]` / `X[expr]` curve-evaluation syntax** in `[Transforms]`, custom equation **`[Constraints]` block** (including mandatory-constant prompts), `"Clone built-in equation"`, persistent cross-project **User Equation Library**, and **`MultipleVariables` unknown interpolation** (retaining blank-$X$ / blank-$Y$ rows in `crates/fit/src/types.rs`).
- [ ] Add the 7 missing modern built-in equations (`Padé (1,1)` approximant, `Horizontal line`, `Semilog X-linear / Y-log`, `Sum of two Lorentzians`, 3 fixed-scale `Cumulative Gaussian` forms) and 12 legacy reparameterizations (for `.pzfx` backward compatibility); resolve the 8 built-in parameterization divergences (`operational_model_agonist` `<A>` vs `<~A>`, `5PL`, `two_sites_fit_ki` `HotKdHi`/`HotKdLo`, `association_kinetics_one_conc`, `allosteric_ec50_shift`, `lorentzian` HWHM vs FWHM, wave `Wavelength`, and `LogisticGrowth`/`GompertzGrowth` inflection-point derived transforms) via clean additive IDs/aliases without breaking existing saved projects.
- [ ] Close custom DSL syntax & diagnostic gaps (`Y' = ...`, `Y[0]`, `[]`/`{}` grouping, `\` line continuation, stepped `<A:J,3>` and multi-letter `<AA>` selectors, variadic `AND`/`OR`, missing aliases, Adjusted/Weighted $R^2$, RMSE, Poisson pseudo-$R^2$/dispersion, `HomoscedasticityPlot`, `ActualVsPredictedPlot`, `ResidualVsRowPlot`), and implement [`curve-fitting-implementation.md`](implementation/curve-fitting-implementation.md) §6.2 multi-dataset Info-hooked continuous ($z, t, F, \chi^2$) tail-shading and discrete (`Binomial`, `Poisson`) PMF plotting (`Plot a function`), backed by `D1` R `4.5.2` and `mpmath` oracles.

---

## R24 Advanced graph formatting, workbench pipelines, layouts & visual audit (post-Beta deferral)

**Goal (explicitly deferred to post-Beta; decided 2026-11-26):** Capture remaining non-blocking visual customization, reusable pipeline, layout/drawing/printing, lab-standardization, and visual-audit backlog cleanly without widening Beta scope.

### R24.1 Advanced graph formatting, axes, scale bars, heatmaps & multivariate visuals (post-Beta)
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 1; deferred post-Beta) · Depends on: **Beta** release · Gate: post-Beta (deferred) · Est. note: 40–65 h

- [ ] Extend `AxisSpec` and `Format Axes`: general reversed axis direction, hidden axes paired with L-shaped **Scale Bars**, `First tick` starting offset, independent axis show/hide, separate Major vs. Minor gridlines, tick length, tick prefix/suffix (with sign placement) & manual decimals, `"Show INSTEAD OF regular ticks"`, hooked `AdditionalTick` (`HookSource`), reference-line z-order (`over data`), Origin Placement presets (`Lower-Left/Right`, `Upper-Left/Right`, `Custom (X0, Y0)`), graph aspect presets (`Tall`/`Wide`/`Square`), and `Format Axes > Titles & Fonts`.
- [ ] Extend `GraphSpec` and `Format Graph`: independent plot-area fill & axis color, custom user-defined named Color Schemes, bar/box/violin/shape fill patterns & hatching, per-dataset heterogeneous Grouped bar arrangements, custom bar baseline $\ne 0$, granular bar spacing %, bar/point value annotations, connecting-line `"gap at symbols"`, XY per-dataset row skipping, subset bulk editing (`Select Datasets…`), reset individual point formatting, independent canvas axis-length drag vs. proportional `Resize Graph`, multi-table compound graph UI (`SeriesSpec::source_sheet_id`), cross-file/recent-donor `ApplyStyleFrom`, and wiring dead visual fields (`ScatterAndLine::area_fill`, `KaplanMeierStep::show_all_points`, `Sheet::color_tag`).
- [ ] Extend Heatmaps, Multivariate graphs, Survival & Post-hoc overlays: heatmap square-cell lock, cell borders, group-separator gaps, 3-stop diverging midpoint, `'×'` missing/excluded marker, and **2D categorical aggregation** (`Count`/`Mean`/`Median`/`SD`/`Sum`); MV bubble sizing modes (smallest-only/largest-only/categorical, clamp-vs-omit, 3-circle & combined fill+size legends, decoupled grouping & configurable CI level on Confidence Ellipses/Convex Hulls); MV categorical summary lines / error bars / before–after strips; `Format Number at Risk Table`; CLD outside-graph/direction/numeric modes; and `PairwiseBracketSpec` custom prefix & split asterisk/text sizes.

### R24.2 Reusable pipelines (`.cbmethod`), sheet freezing, page layout objects, drawing, printing & lab preferences (post-Beta)
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 6; deferred post-Beta) · Depends on: **Beta** release · Gate: post-Beta (deferred) · Est. note: 32–52 h

- [ ] Add an apply-pipeline-to-populated-table workflow and reusable `.cbmethod` files (applying analysis + graph chains to already-populated data tables, distinct from blank `.cbtemplate` and style-only `graph.applyStyleFrom`), plus `frozen: bool` snapshot freezing on `ResultSheet` and `GraphSheet`.
- [ ] Add missing Page Layout & drawing tools: **Master Legends** (shared across panels), unlinked static graph snapshots, 90° rotated depth-chart panels, bulk `Resize Layout`, double-headed/curved arrows, capped lines, text-centered brackets/zigzags, arrowhead styles/sizes, object rotation/group/ungroup/lock, Distribute / Center on Page / exact Position Object `(X, Y, angle)`, ruler guides, and reverse legend order.
- [ ] Add paginated Data/Info/Results table printing, Gallery contact-sheet printing, pure-black print mode, tiled multi-page poster printing, cell background highlighting, extended raster/vector export formats (CMYK TIFF, 8-bit indexed RGB / PackBits TIFF, Grayscale SVG, multi-page PDF batch bundling, export-time width scaling), and lab standardization (read-only locked teaching-lab prefs, shared lab templates/methods/equation folders, and rich application defaults beyond `settings.md`).

### R24.3 Workflow specification backlog & full visual parity audit (post-Beta)
Owner: open · Status: todo · Was: reference-coverage audit 2026-11-26 (Cluster 7; deferred post-Beta) · Depends on: **Beta** release · Gate: post-Beta (deferred) · Est. note: 16–28 h

- [ ] Write clean-room functional specifications for remaining un-specced Series 02–06 statistical/graphing workflows (including contingency `1/OR` + Row/Col/Total % sub-tables + $N > 1{,}000{,}000$ automatic Fisher-to-Chi-square fallback, and linear regression `ReplicateFitMode`).
- [ ] Complete the full visual parity audit beyond the 13-frame pilot across desktop surfaces (`egui 0.37`), recording neutral clean-room findings and triaging any actionable regressions into follow-up items.

## R25 Imported analysis live recompute & interop accuracy

**Goal:** Turn the `.prism` analysis verification from the [`R22.5`](history/roadmap/r22.md#r225-external-archive-import-corpus-verified-translation--product-integration-beta-gate) follow-up ([#69](https://github.com/HauserGroup/curvebook/pull/69), [#70](https://github.com/HauserGroup/curvebook/pull/70)) into user-visible behaviour. Analysis designs that agree with the cached results across the external corpus recompute live on open, with a per-file warning when a recomputed value differs from the cached one. Every other design stays a read-only snapshot. Then close the accuracy, translation and interop gaps the corpus exposed. Cached `.prism` values stay a consistency check, never ground truth (`D1`).

### R25.1 Promote corpus-verified imported analyses to live recompute (Beta gate)
Owner: open · Status: done (2026-11-27) · Was: `R22.5` follow-up 2026-11-27 · Depends on: [#69](https://github.com/HauserGroup/curvebook/pull/69), [#70](https://github.com/HauserGroup/curvebook/pull/70) merged · Gate: **Beta** · Est. note: 20–32 h

- [x] **Per-design verification table.** Replace the class-level `AnalysisMappingRow::verification` (`analyses.rs` in the `.prism` archive module of `crates/format`) with a declarative table of verified designs keyed by `analysisClass`, `experimentalDesign`, tail (`pValueTailType`) and post-test (`postTests` family and test). `translate_analysis_parameters` returns `CorpusVerified` only for a listed design with no blocking options. Each row records its corpus evidence (analyses compared, distinct source records, agreement) and the date. Promotion bar: at least 20 analyses compared from at least 2 source records, at least 99% fully agreeing, and every remaining disagreement explained, either as a stale cache (recomputed from the file's own `data.csv`) or as a documented method difference.
- [x] **Pairwise crosswalk.** Extend `CACHED_SCALAR_LINKS` to post-test results: match cached pairwise entries to `ResultSheet::pairwise_comparisons` by sheet data-set pair, and compare the adjusted p-value and the magnitude of the mean difference. ANOVA designs with post-tests are promoted only once their pairwise values also clear the bar.
- [x] **Harness evidence and regression gate.** `cargo xtask interop-corpus` records distinct source records per design next to the existing per-design counts. `--strict` fails when a design in the verified table falls below the bar, so later stats changes cannot silently regress promoted designs.
- [x] **Live import.** For verified designs the importer builds a live `ResultSheet` with the resolved `selected_data_sets`. It keeps the crosswalked cached values as transient reference data in `ImportReport`, never persisted in `.curvebook` files. Other designs keep today's `ResultProvenance::Imported` snapshot.
- [x] **Per-file disagreement warning.** After the first recalculation, on both open paths (project load in `crates/engine/src/services/formats.rs` and `table.importPrism`, including `merge: true` with `SheetId` renumbering), compare each promoted analysis with its reference values. Emit a typed `ImportWarning` naming the analysis title, the quantity, and both values, for example "recomputed p = 0.0037, file stored p = 0.156". Surface it in the desktop import report, CLI output and MCP outcome warnings. CurveBook's value always stands.
- [x] **Snapshot fallback.** When a promoted analysis fails to recompute on import (degenerate inputs such as a two-group test with one data set), keep the cached snapshot (`ImportedSnapshot` with its result tabs) and add a warning, instead of showing a failed result sheet.
- [x] **Tests.** Format: table tests for the verified-design lookup. Engine: a minimal synthetic `.prism` fixture with an agreeing promoted analysis, a stale-cache disagreement and a failing recompute, run through both open paths and a merge. If the GUI import report changes, add a five-stage `WorkflowRun` human-path test per [`pitfalls.md`](pitfalls.md) §6–§7.
- [x] **Docs.** Update `docs/open-pzfx-files.html`, `docs/faq.html` and `docs/reference/files.md` (verified designs recompute on open, plus what the warning means), [`STATUS.md`](STATUS.md), and `docs/evidence/interop-format-notes.md` (verified-design evidence). Keep corpus counts out of hand-written prose (pitfall §2).
- Promoted on 2026-11-27 (all 10 candidates cleared the bar and pairwise check; evidence in [`evidence/interop-format-notes.md`](evidence/interop-format-notes.md)): two-tailed unpaired, paired and Welch *t*-tests; ordinary one-way ANOVA with no post-test, Tukey (with `1e-5` external-cache tail-quadrature tolerance on pairwise adjusted *p*) or Dunnett; ordinary two-way ANOVA with interaction and no post-test (`includeInteractionTerm: false` blocked); descriptive statistics (`SUB_COLUMN` blocked); area under the curve; and outlier detection on single-subcolumn or pooled replicate tables (`SUB_COLUMN` and multi-subcolumn `AVERAGE` blocked). Remaining designs (Mann–Whitney, Kruskal–Wallis, repeated-measures two-way, nonlinear fits, normality tests, etc.) stay read-only snapshots (see [`R25.2`](#r252-imported-analysis-accuracy-gaps-post-beta)–[`R25.3`](#r253-imported-analysis-translation-coverage-post-beta)).

### R25.2 Imported analysis accuracy gaps (post-Beta)
Owner: open · Status: todo · Was: `R22.5` follow-up corpus sweep 2026-11-27 · Depends on: [#69](https://github.com/HauserGroup/curvebook/pull/69) · Gate: post-Beta · Est. note: 24–48 h

Each issue explains its disagreements against published references and either fixes CurveBook, translates a missing option, or records a documented method difference. Never tune CurveBook output to match cached values.

- [ ] Mann-Whitney p-values ([#71](https://github.com/HauserGroup/curvebook/issues/71)).
- [ ] Kruskal-Wallis with Dunn post-tests ([#72](https://github.com/HauserGroup/curvebook/issues/72)).
- [ ] Two-way repeated-measures ANOVA, including the untranslated sphericity setting ([#73](https://github.com/HauserGroup/curvebook/issues/73)).
- [ ] Nonlinear regression constraints, initial values and outlier settings ([#74](https://github.com/HauserGroup/curvebook/issues/74)).
- [ ] Normality-test statistics as result scalars, so they can be verified ([#75](https://github.com/HauserGroup/curvebook/issues/75)).

### R25.3 Imported analysis translation coverage (post-Beta)
Owner: open · Status: todo · Was: `R22.5` follow-up corpus sweep 2026-11-27 · Depends on: [#69](https://github.com/HauserGroup/curvebook/pull/69); pairwise crosswalk from [`R25.1`](#r251-promote-corpus-verified-imported-analyses-to-live-recompute-beta-gate) for [#77](https://github.com/HauserGroup/curvebook/issues/77) · Gate: post-Beta · Est. note: 16–30 h

- [ ] Selected-pairs post-test families ([#76](https://github.com/HauserGroup/curvebook/issues/76)).
- [ ] Mapping of the `BONFERRONI_SIDAK` post-test identifier ([#77](https://github.com/HauserGroup/curvebook/issues/77)).
- [ ] Control-vs-all post-tests beyond one-way Dunnett ([#78](https://github.com/HauserGroup/curvebook/issues/78)).
- [ ] Data-transform functions written as expressions ([#79](https://github.com/HauserGroup/curvebook/issues/79)).

### R25.4 Interop follow-ups: saving, rebuilt graphs & post-test cost (post-Beta)
Owner: open · Status: todo · Was: compatibility scorecard review 2026-11-27 · Depends on: none · Gate: post-Beta · Est. note: 16–28 h

- [ ] Imported projects that cannot be saved as `.curvebook` files because of an archive security limit ([#80](https://github.com/HauserGroup/curvebook/issues/80)).
- [ ] Basic editable graphs rebuilt from each `.prism` graph's `inputDataSets`, without decoding binary graph blobs ([#81](https://github.com/HauserGroup/curvebook/issues/81)).
- [ ] Per-pair p-value cost in large post-test families ([#82](https://github.com/HauserGroup/curvebook/issues/82)).

---

## Order and dependencies

Active tracks only; see [earlier sequencing](history/roadmap.md#earlier-sequencing) for completed work.

| Track | Open items | Depends on | Target gate |
|---|---|---|---|
| **Beta hosted checks & rehearsal** | [`R0.36`](#r036-full-workspace-gap--delta-audit-remediation-beta-gate), [`R14.1`](#r141-beta-release-pipeline), [`R0.27`](#r027-user-first-readme-overhaul-beta-gate), [`R12.1`](#r121-documentation-site-on-github-pages) | Apple credentials (configured); local code complete (incl. [`R25.1`](#r251-promote-corpus-verified-imported-analyses-to-live-recompute-beta-gate) done locally) | **Beta** |
| **Optional pre-Beta analytical wave** | [`R23.2`](#r232-complete-lognormal-hypothesis-testing-suite--standalone-normalitylognormality)–[`R23.6`](#r236-nonlinear-regression-constraints-derived-ci-transforms-equation-catalog-dsl-depth--theoretical-plotting-62) (`R23.1` archived in [`r23.md`](history/roadmap/r23.md)) | Coordinates with [`R19.2`](history/roadmap/r19.md#r192-high-usage-statistical-analyses--graph-families) (done) | Optional before **Beta** |
| **Post-Beta distribution & citation** | [`R14.4`](#r144-homebrew-tap-for-macos-post-beta), [`R14.5`](#r145-mcp-bundle-mcpb-and-mcp-registry-listing-post-beta), [`R14.7`](#r147-software-citation-citationcff-zenodo-doi--versioned-methods-paragraph-post-beta) | [`R14.1`](#r141-beta-release-pipeline) | Post-**Beta** |
| **Post-Beta guides & search** | [`R12.2`](#r122-example-workflow-guides), [`R12.3`](#r123-functionality-and-methods-reference), [`R12.7`](#r127-application-user-guide-desktop-and-browser), [`R12.8`](#r128-search-discoverability-for-the-pages-site) | `R12.1`, **Beta** release (`R12.7` browser part waits for `R11.10`) | Post-**Beta** / **1.0** |
| **Post-Beta polish & external QA** | [`R8.13`](#r813-older-macos-metal-device-lost-hardening--windows-console-suppression), [`R10.31`](#r1031-desktop-graph-image-gallery--linked-region-ui-parity-fixes)–[`R10.33`](#r1033-advanced-importpaste-special-live-link-persistence--export-figure-options), [`R17.2`](#r172-downstream-vector-and-document-export-interoperability)–[`R17.4`](#r174-live-llm-mcp-usability-trials-and-ergonomics-fixes) | Independent (external apps/hardware for `R17.2`–`R17.4`) | Post-**Beta** |
| **Post-Beta analytical & graphing backlog** | [`R24.1`](#r241-advanced-graph-formatting-axes-scale-bars-heatmaps--multivariate-visuals-post-beta)–[`R24.3`](#r243-workflow-specification-backlog--full-visual-parity-audit-post-beta) | **Beta** release for `R24` (`R19.1`–`R19.7` done) | Post-**Beta** |
| **Post-Beta imported-analysis accuracy & interop** | [`R25.2`](#r252-imported-analysis-accuracy-gaps-post-beta)–[`R25.4`](#r254-interop-follow-ups-saving-rebuilt-graphs--post-test-cost-post-beta) | [#69](https://github.com/HauserGroup/curvebook/pull/69); [`R25.1`](#r251-promote-corpus-verified-imported-analyses-to-live-recompute-beta-gate) pairwise crosswalk done locally for [#77](https://github.com/HauserGroup/curvebook/issues/77) | Post-**Beta** |
| **Engineering hygiene** | [`R0.16`](#r016-local-source-coverage-reports-cargo-llvm-cov), [`R0.35`](#r035-keep-the-ci-cache-under-the-repository-budget), [`R14.2`](#r142-release-sbom-and-ci-runner-hardening), [`R14.3`](#r143-retry-transient-release-verification-failures) | `R14.1` for `R14.2`–`R14.3` | none |
| **1.0 desktop QA, signing & translations** | [`R10.13`](#r1013-human-operable-50-workflow-acceptance--manual-qa), [`R10.24`](#r1024-windows--linux-desktop-gui-smoke-qa-10-gate), [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening), [`R8.11`](#r811-windows-authenticode-signing-after-beta), [`R8.12`](#r812-desktop-rtl-gui-mirroring--native-speaker-translation-review-10-gate) (`ar`/`he` RTL unlikely) | [`R14.1`](#r141-beta-release-pipeline) (`R11.10` for `R10.13` browser part) | **1.0** |
| **1.0 browser app (`eframe::WebRunner`)** | [`R11.1`](#r111-real-browser-test-harness--web-ci-gates) -> [`R11.11`](#r1111-migrate-browser-host-to-eframe-webrunner-10-gate) -> `R11.2`–`R11.9` -> [`R11.10`](#r1110-live-browser-qa--supported-browser-matrix) | Deferred post-**Beta** | **1.0** |
| **Microsoft Store (MSIX)** | [`R8.9`](#r89-windows-msix-packaging--microsoft-store-distribution) | Packaging independent; submission needs `R14.1`, `R8.5`, `R10.13` | Store channel |

**Open decisions:** `R11.1` browser test runner (`wasm-bindgen-test` recommended) and `R11.6` Web Worker vs. chunked main-thread compute; `R0.36` Cividis ΔL* stop-band tolerance; `R14.2` CI runner egress allowlist.

---

## Legacy Milestone & Gate Mapping

Legacy `G1`–`G14`, `Slice 0`, `S1`–`S2`, `M0`–`M8+`, and `#known-defects` records live in [`history/legacy-ids-and-defects.md`](history/legacy-ids-and-defects.md).

### Release Gates

Under [`R0.21`](history/roadmap/r0.md#r021-strict-alpha-branding-and-asset-provenance-gate), [`R12.8`](#r128-search-discoverability-for-the-pages-site), and [`R10.34`](history/roadmap/r10.md#r1034-main-gui-file-extension-naming-for-imported-project-files-beta-gate), lowercase `".pzfx files"` and `".prism files"` are allowed and preferred on user-facing surfaces; all other third-party names stay excluded (`cargo xtask brands`).

| Release Gate | Required Items | Status |
|---|---|---|
| **Alpha** | `R7.3`, `R7.4`, `R0.20`, `R0.21` | **Released 2026-11-17** as [`v0.1.0-alpha.1`](https://github.com/HauserGroup/curvebook/releases/tag/v0.1.0-alpha.1). |
| **Agent-harness workflows** | `R8.3`, `R8.6`–`R8.8` | **Done (historical).** Replaced by five-stage `EguiHumanPathDriver` runs after `--gui-only` retirement (`R21.1`). |
| **Human GUI workflows** | `R10.23`, `R15.1`–`R15.10`, `R13.1`–`R13.10`, `R10.25`–`R10.28` (**Beta**); `R10.13`, `R10.24` (**1.0**) | **Beta automated gate done (`65 / 65` complete).** Recorded human/computer-use walkthroughs (`R10.13`, `R10.24`) remain **1.0**. |
| **Beta** | **Alpha** + `R10.23`, `R10.25`–`R10.30`, `R10.34`, `R13`, `R15`, `R20.19`, `R5.4`, `R22`, `R12.4`–`R12.5` (done on `main`) + [`R25.1`](#r251-promote-corpus-verified-imported-analyses-to-live-recompute-beta-gate) (done locally) + [`R0.36`](#r036-full-workspace-gap--delta-audit-remediation-beta-gate), [`R14.1`](#r141-beta-release-pipeline), [`R0.27`](#r027-user-first-readme-overhaul-beta-gate), [`R12.1`](#r121-documentation-site-on-github-pages) | **Local code complete** (incl. `R25.1`); **hosted checks open:** green `ci.yml` on `main`, Windows `.msi` `release-scan` confirmation, one `v0.1.0-alpha.2` rehearsal release + automated clean-runner macOS/Windows checks (`R14.1`, `R0.36`), scripted README check (`R0.27`), and automated live Pages verification (`R12.1`). Zero manual human steps. |
| **1.0** | **Beta** + [`R10.13`](#r1013-human-operable-50-workflow-acceptance--manual-qa), [`R10.24`](#r1024-windows--linux-desktop-gui-smoke-qa-10-gate), [`R8.5`](#r85-cross-os-hardware-cjkrtl-desktop-qa--installersigning-hardening), [`R8.11`](#r811-windows-authenticode-signing-after-beta), [`R8.12`](#r812-desktop-rtl-gui-mirroring--native-speaker-translation-review-10-gate) (LTR/CJK native review; `ar`/`he` RTL unlikely), [`R11`](#r11-usable-browser-app) | Open for full release after Beta. |

---

## Parked

Items explicitly deferred or unlikely:

- **Arabic (`ar`) & Hebrew (`he`) desktop RTL GUI mirroring:** **unlikely / unscheduled** (`fa`/`ur` likewise unscheduled); existing `ar`/`he` catalog columns remain hidden behind `"en"` fallback in `normalize_beta_gui_locale` ([`R8.12`](#r812-desktop-rtl-gui-mirroring--native-speaker-translation-review-10-gate)).
- **Translated READMEs:** deferred beyond initial `1.0` (`R0.27`).
- **Plugin SDK / dynamic plugins:** deferred (`L0`–`L7` workspace stays self-contained with `#![forbid(unsafe_code)]`).
- **Multi-project in-window document tabs & multi-window tiling:** deferred post-`1.0` ([`gui-layout-spec.md`](planning/gui-layout-spec.md) §3.1).
- **External public-source teaching-workflow & pain-point corpus mining:** deferred (local corpora stay outside the tracked repository).

---

## Completed roadmap items

Each item ID links directly to its archived section and verification record.

| Milestone archive | Completed items |
|---|---|
| [R0 Align with house principles](history/roadmap/r0.md#r0-align-with-house-principles) | [`R0.1`](history/roadmap/r0.md#r01-governance-agent--contributor-docs), [`R0.2`](history/roadmap/r0.md#r02-claimable-roadmap--status-conversion), [`R0.3`](history/roadmap/r0.md#r03-secrets-gate--pre-commit-hook), [`R0.4`](history/roadmap/r0.md#r04-roadmap-pending--status-tooling), [`R0.5`](history/roadmap/r0.md#r05-licences--supply-chain-policy), [`R0.6`](history/roadmap/r0.md#r06-repository-hygiene--ci-cleanup) *(merged)*, [`R0.7`](history/roadmap/r0.md#r07-generated-command--schema-docs), [`R0.8`](history/roadmap/r0.md#r08-typed-commandspec-registry), [`R0.9`](history/roadmap/r0.md#r09-structured-engineerror--checklistitem-remediation), [`R0.10`](history/roadmap/r0.md#r010-curvebook-platform-safe-process--atomic-writes), [`R0.11`](history/roadmap/r0.md#r011-versioned-config-journal--lock-files), [`R0.12`](history/roadmap/r0.md#r012-format-importer--exporter-registry), [`R0.13`](history/roadmap/r0.md#r013-mcp-cli--ui-alignment-over-the-typed-registry), [`R0.14`](history/roadmap/r0.md#r014-release-verification-scripts--readme-scaffold) *(merged)*, [`R0.15`](history/roadmap/r0.md#r015-evidence-backed-done-markers), [`R0.17`](history/roadmap/r0.md#r017-clean-room-provenance-review-lilliefors-p-values--bessel-functions), [`R0.18`](history/roadmap/r0.md#r018-upgrade-the-workspace-and-build-toolchains-to-rust-1991), [`R0.19`](history/roadmap/r0.md#r019-round-2-delta-code-audit-remediation-ebcc622fda1cc0), [`R0.20`](history/roadmap/r0.md#r020-rename-lumencraft-to-curvebook), [`R0.21`](history/roadmap/r0.md#r021-strict-alpha-branding-and-asset-provenance-gate), [`R0.22`](history/roadmap/r0.md#r022-supply-chain-advisory-coverage-and-focused-contributor-guides), [`R0.24`](history/roadmap/r0.md#r024-bounded-tcp-fixtures-and-staged-ci) *(merged)*, [`R0.25`](history/roadmap/r0.md#r025-r0r9-milestone-review-ledger-remediation-2026-11-16), [`R0.26`](history/roadmap/r0.md#r026-hosted-release-asset-branding--provenance-scan) *(merged)*, [`R0.28`](history/roadmap/r0.md#r028-rein-in-the-number-of-release-install-files) *(merged)*, [`R0.29`](history/roadmap/r0.md#r029-rustdoc-link-gate), [`R0.30`](history/roadmap/r0.md#r030-api-documentation-for-numerical-and-model-crates), [`R0.31`](history/roadmap/r0.md#r031-docs-folder-consolidation), [`R0.32`](history/roadmap/r0.md#r032-fast-local-ci-tier-cargo-xtask-ci-quick), [`R0.33`](history/roadmap/r0.md#r033-issue-driven-bug-fix-loop), [`R0.34`](history/roadmap/r0.md#r034-checked-lock-wrapper-for-shared-state) |
| [R1 Workspace, numerical foundations & NIST StRD oracle](history/roadmap/r1.md#r1-workspace-numerical-foundations--nist-strd-oracle) | [`R1.1`](history/roadmap/r1.md#r11-layered-workspace--clean-room-quality-gates), [`R1.2`](history/roadmap/r1.md#r12-ieee-754-foundations-special-functions-linear-algebra--nist-strd--d1-corpus) |
| [R2 Typed data tables, reactive DAG, persistence & legacy import](history/roadmap/r2.md#r2-typed-data-tables-reactive-dag-persistence--legacy-import) | [`R2.1`](history/roadmap/r2.md#r21-palettes-8-typed-table-archetypes-subcolumns--dependency-links), [`R2.2`](history/roadmap/r2.md#r22-curvebook-archive-wal-crash-recovery--curvebookd-unpacked-format), [`R2.3`](history/roadmap/r2.md#r23-reactive-recalculation-dag-bounded-undoredo--linked-file-error-propagation), [`R2.4`](history/roadmap/r2.md#r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget) |
| [R3 Core biostatistics, ANOVA/REML, survival, contingency, ROC & multivariate](history/roadmap/r3.md#r3-core-biostatistics-anovareml-survival-contingency-roc--multivariate) | [`R3.1`](history/roadmap/r3.md#r31-descriptive-statistics-normality-battery-outliers--two-group-tests), [`R3.2`](history/roadmap/r3.md#r32-one-two-three-way-anova-rm-reml-mixed-models--multiplicity), [`R3.3`](history/roadmap/r3.md#r33-survival-cox-ph-contingency-barnardkoopman-method-comparison-roc--glmspcamice), [`R3.4`](history/roadmap/r3.md#r34-unstructured-residual-covariance-for-reml-mixed-models), [`R3.5`](history/roadmap/r3.md#r35-statistical-method-fidelity-fixes-from-the-d1-citation-pass), [`R3.6`](history/roadmap/r3.md#r36-mixed-effects-reml-for-1-way--3-way-rm-and-selectable-df-methods), [`R3.7`](history/roadmap/r3.md#r37-cox-ph-exact-ties-per-risk-set-stability--large-span-baptistapike-cis) |
| [R4 Nonlinear regression solver, equation library & custom DSL/ODE](history/roadmap/r4.md#r4-nonlinear-regression-solver-equation-library--custom-dslode) | [`R4.1`](history/roadmap/r4.md#r41-damped-lm-solver-weighting-robustrout-shared-globals-profile-cis--aiccflrt), [`R4.2`](history/roadmap/r4.md#r42-built-in-equation-library-117-models-across-all-categories), [`R4.3`](history/roadmap/r4.md#r43-safe-custom-algebraicimplicitode-dsl-bytecode-vm), [`R4.4`](history/roadmap/r4.md#r44-derived-table-transforms), [`R4.5`](history/roadmap/r4.md#r45-suppress-least-squares-inference-in-pure-robust-fit-mode), [`R4.6`](history/roadmap/r4.md#r46-fitting-method-fidelity-fixes-from-the-d1-citation-pass) |
| [R5 Publication graphing, vector export & Page Layouts](history/roadmap/r5.md#r5-publication-graphing-vector-export--page-layouts) | [`R5.1`](history/roadmap/r5.md#r51-scientific-scene-compiler-rich-typography-nonlinearbroken-axes--brackets), [`R5.2`](history/roadmap/r5.md#r52-publication-svg-pdf-17--fontfile2-pngtiff--ecma-234-emf-export), [`R5.3`](history/roadmap/r5.md#r53-headless-page-layouts-style-cloning-spool-pdf--subprocessmacos-clipboard), [`R5.4`](history/roadmap/r5.md#r54-native-binary-os-vector-clipboard-interactive-print-dialogs--live-layout-drag-snap) |
| [R6 Multiple Variables workflows, Calculated Variables & Multifactor ANOVA](history/roadmap/r6.md#r6-multiple-variables-workflows-calculated-variables--multifactor-anova) | [`R6.1`](history/roadmap/r6.md#r61--calculated-variables-role-locking--level-ordering), [`R6.2`](history/roadmap/r6.md#r62-multifactor-anova-mv-groupingsplits--five-channel-mv-plots), [`R6.3`](history/roadmap/r6.md#r63-window-clauses-over-where-order-by-rows--conditional-aggregations) |
| [R7 Headless automation, CLI, `--control`, MCP & performance budgets](history/roadmap/r7.md#r7-headless-automation-cli---control-mcp--performance-budgets) | [`R7.1`](history/roadmap/r7.md#r71-slice-0-headless-cli---control-tcp-jsonl--mcp-server), [`R7.2`](history/roadmap/r7.md#r72-performance-budgets-1m-cell-virtualization--propertyfuzz-suites), [`R7.3`](history/roadmap/r7.md#r73-mcp-workflow-parity-proof--shared-live-window), [`R7.4`](history/roadmap/r7.md#r74-accessible-mcp-guide-and-minimal-readme) |
| [R8 Desktop & WebAssembly live shell, localization & `1.0` hardening](history/roadmap/r8.md#r8-desktop--webassembly-live-shell-localization--10-hardening) | [`R8.1`](history/roadmap/r8.md#r81-pure-rust-uiapp-shell-software-rgba-compositor-hidpirtl-input--webworkspacehost), [`R8.2`](history/roadmap/r8.md#r82-16-locale-i18n-rtl-mirroring--desktop-cjkrtl-fontpasteboard-checks), [`R8.3`](history/roadmap/r8.md#r83-live-os-eframeeguiwgpu-desktop-windowing--interactive-event-loop), [`R8.4`](history/roadmap/r8.md#r84-live-wasm32-unknown-unknown-trunk-webgpuwebgl2-browser-hosting), [`R8.6`](history/roadmap/r8.md#r86-semantic-ui-inspection-gui-mcp-toolset--strict---gui-only-lock), [`R8.7`](history/roadmap/r8.md#r87-interactive-power--sample-size-wizard-dynamic-modal-predicates--chained-gui-surfaces), [`R8.8`](history/roadmap/r8.md#r88-clean-room-28-workflow-gui_only-acceptance-suite), [`R8.10`](history/roadmap/r8.md#r810-signed-macos-installers--download-verification-beta-gate) *(merged)* |
| [R9 Post-parity scientific quality-of-life suite](history/roadmap/r9.md#r9-post-parity-scientific-quality-of-life-suite) | [`R9.1`](history/roadmap/r9.md#r91-non-destructive-table-archetype-conversion-qol-1), [`R9.2`](history/roadmap/r9.md#r92-96384-well-microplate-mapper--tidy-long-to-wide-pivot-qol-2), [`R9.3`](history/roadmap/r9.md#r93-auditable-cell-exclusion-reasons--provenance-audit-qol-3), [`R9.4`](history/roadmap/r9.md#r94-optional-side-by-side-split-workspace-qol-4), [`R9.5`](history/roadmap/r9.md#r95-two-way-data-to-graph-point-brushing-qol-5), [`R9.6`](history/roadmap/r9.md#r96-one-click-methods--statistical-reporting-generator-qol-6), [`R9.7`](history/roadmap/r9.md#r97-inline-assumption-diagnostic-banners-diagnostics-only-qol-7), [`R9.8`](history/roadmap/r9.md#r98-journal-figure-dimension--typography-lock-presets-qol-8), [`R9.9`](history/roadmap/r9.md#r99-live-color-vision-deficiency--grayscale-canvas-preview-qol-9), [`R9.12`](history/roadmap/r9.md#r912-categorical--continuous-table-analysis-completions-qol-12), [`R9.14`](history/roadmap/r9.md#r914-experimental-subject-randomization--random-subset-sampling-qol-14), [`R9.16`](history/roadmap/r9.md#r916-live-best-fit-tex-equation-annotations-equations-only-qol-16), [`R9.17`](history/roadmap/r9.md#r917-extended-multivariable-clustering-glm--statistical-graphing-gui_only-acceptance-suite-qol-17) |
| [R10 Human-operable GUI & 50-workflow human completion](history/roadmap/r10.md#r10-human-operable-gui--50-workflow-human-completion) | [`R10.1`](history/roadmap/r10.md#r101-interactive-chrome-architecture-decision--spike)–[`R10.12`](history/roadmap/r10.md#r1012-import--paste-special-and-linked-region-provenance), [`R10.14`](history/roadmap/r10.md#r1014-external-branding-exclusion-sweep--gate)–[`R10.23`](history/roadmap/r10.md#r1023-close-the-remaining-desktop-gui-gaps-beta-gate), [`R10.25`](history/roadmap/r10.md#r1025-automated-gui-completion-of-all-65-workflows-beta-gate)–[`R10.30`](history/roadmap/r10.md#r1030-human-path-tests-for-import-and-export-beta-gate), [`R10.34`](history/roadmap/r10.md#r1034-main-gui-file-extension-naming-for-imported-project-files-beta-gate), [`R10.35`](history/roadmap/r10.md#r1035-portfolio-gallery-live-thumbnails-batch-workflows--navigation-overhaul), [`R10.36`](history/roadmap/r10.md#r1036-project-navigator-collapsed-rail--section-vector-icons) |
| [R12 User documentation](history/roadmap/r12.md#r12-user-documentation) | [`R12.4`](history/roadmap/r12.md#r124-automation-reference-mcp-cli-control-protocol-and-batch), [`R12.5`](history/roadmap/r12.md#r125-getting-started-files-settings-and-support), [`R12.6`](history/roadmap/r12.md#r126-contributor-documentation-on-pages) *(dropped)*, [`R12.9`](history/roadmap/r12.md#r129-publish-only-the-site-on-pages)–[`R12.14`](history/roadmap/r12.md#r1214-manual-rendering--layout-bug-fixes) |
| [R13 Advanced regression, composite graphing & layout objects](history/roadmap/r13.md#r13-advanced-regression-diagnostics-composite-graphing-layout-objects--65-workflow-human-completion) | [`R13.1`](history/roadmap/r13.md#r131-domain-restricted-regression-fit-identifiability-replicate-unknown-interpolation--outlier-overlays)–[`R13.10`](history/roadmap/r13.md#r1310-missing-format-graph-controls-and-65-workflow-control-gap-sweep) |
| [R14 Releases & distribution](history/roadmap/r14.md#r14-releases--distribution) | [`R14.6`](history/roadmap/r14.md#r146-alpha-to-beta-project-upgrade-test) |
| [R15 Modern egui desktop GUI overhaul (Beta gate)](history/roadmap/r15.md#r15-modern-egui-desktop-gui-overhaul-beta-gate) | [`R15.1`](history/roadmap/r15.md#r151-duplicate-menu-bar-suppression-backend-switch-accesskit-inspection-cpu-screenshot-fallback-driver-and-fontlocale-gates)–[`R15.10`](history/roadmap/r15.md#r1510-scientific-visual-design-language-derived-oklaboklch-tokens--structural-hierarchy-gates) |
| [R16 Core- and GUI-crate maintainability](history/roadmap/r16.md#r16-core--and-gui-crate-maintainability-no-gate) | [`R16.2`](history/roadmap/r16.md#r162-split-the-fit-solver-and-model-dsl-modules)–[`R16.19`](history/roadmap/r16.md#r1619-replace-egui-combo-boxes-with-a-dropdown-that-can-grow-while-open) |
| [R17 Post-Beta adoption, interoperability & real-world polish](history/roadmap/r17.md#r17-post-beta-adoption-interoperability--real-world-polish) | [`R17.1`](history/roadmap/r17.md#r171-direct-xlsx-import-os-drag-and-drop-and-messy-spreadsheet-clipboard-paste), [`R17.5`](history/roadmap/r17.md#r175-real-world-legacy-project-import-hardening), [`R17.6`](history/roadmap/r17.md#r176-report-xlsx-formulas-without-stored-results), [`R17.7`](history/roadmap/r17.md#r177-multi-block-tabular-import-standalone-auc-analysis-and-composable-microplate-mapping) |
| [R19 Statistical, fitting & `D1` oracle breadth](history/roadmap/r19.md#r19-statistical-fitting--d1-oracle-breadth) | [`R19.1`](history/roadmap/r19.md#r191-descriptive-statistics-resultui-parity--nonlinear-identifiability-formatting), [`R19.2`](history/roadmap/r19.md#r192-high-usage-statistical-analyses--graph-families), [`R19.3`](history/roadmap/r19.md#r193-remaining-d1-r-base-statistical-oracles--nist-geardat-worked-examples), [`R19.4`](history/roadmap/r19.md#r194-methods-report-observation-accounting), [`R19.5`](history/roadmap/r19.md#r195-keep-pending-fits-out-of-exported-figures), [`R19.6`](history/roadmap/r19.md#r196-fix-hougaard-skewness-sign-mismatch-and-false-zero-for-uncomputed-entries), [`R19.7`](history/roadmap/r19.md#r197-nonlinear-fit-report-clarity-per-dataset-goodness-of-fit--null-model-diagnostics) |
| [R20 Desktop GUI visual system & layout overhaul](history/roadmap/r20.md#r20-desktop-gui-visual-system--layout-overhaul) | [`R20.1`](history/roadmap/r20.md#r201-visual-design-system--layout-specification-overhaul)–[`R20.22`](history/roadmap/r20.md#r2022-apple-hig-alertmodal-button-order-macos-window-menu--menu-bar-parity) |
| [R21 Structural LOC, test-layer & documentation reduction](history/roadmap/r21.md#r21-structural-loc-test-layer--documentation-reduction-no-gate) | [`R21.1`](history/roadmap/r21.md#r211-retire---gui-only-and-synthetic-ui-automation-plumbing)–[`R21.4`](history/roadmap/r21.md#r214-reduce-documentation-footprint-audit-bloat-and-generated-files) |
| [R22 External scientific file compatibility & project merging](history/roadmap/r22.md#r22-external-scientific-file-compatibility--project-merging-beta-gate) | [`R22.1`](history/roadmap/r22.md#r221-external-interop-corpus-harness-and-real-file-import-defect-fixes-beta-gate)–[`R22.6`](history/roadmap/r22.md#r226-generic-curvebook-project-merging-consolidated-sheetid-renumbering--navigator-origin-badges-beta-gate) |
| [R23 Analytical, statistical, regression & formula breadth](history/roadmap/r23.md#r23-analytical-statistical-regression--formula-breadth) | [`R23.1`](history/roadmap/r23.md#r231-calculated-variables-formula-engine-parity-bug-fixes--window-semantics) |
