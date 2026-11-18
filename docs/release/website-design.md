# Website and GitHub Pages design decisions

Decisions made: 2026-11-16–17. Record clarified: 2026-11-18. Scope: the static GitHub Pages entry point in [`index.html`](../index.html) and dedicated privacy policy in [`privacy.html`](../privacy.html), styled by [`site.css`](../site.css). Publication is separate; see [`github-pages.md`](github-pages.md).

## Scope

This record governs the public CurveBook website and its GitHub Pages presentation: page structure, visitor-facing copy, visual styling, links, accessibility, and publication. It does not prescribe the desktop application interface, browser application interface, application themes, menus, dialogs, or scientific graph styles. References to windows, title bars, or desktop styling below describe the website's visual treatment only.

## Rationale and choices made

- **Focused landing page plus dedicated privacy policy:** keep the product introduction, complete-workspace sharing, and downloads together on [`index.html`](../index.html), accompanied by a dedicated plain-language privacy policy at [`privacy.html`](../privacy.html) for Microsoft Store and distribution disclosures. One primary action and three navigation links give visitors a short path to the essentials. Detailed setup and reference material stay in the existing documentation.
- **Scientific-software identity:** describe CurveBook as an open-source, clean-room biostatistics, nonlinear regression, and scientific graphing application written from scratch in pure Rust. The landing page should introduce the product; MCP belongs in the setup and release-status context.
- **Workspace sharing as a central benefit:** make the saved `.curvebook` project prominent rather than presenting sharing as a minor export option. Explain the single-file mechanism and preserve the distinction from live collaboration.
- **Classic coral over the turquoise proposals:** the earlier turquoise direction made too much of the page feel teal. Warm-grey chrome and white document surfaces provide a quiet base; slate title bars and a restrained coral action give the page character. Keep the logo's spectrum intact and use its warm colors as inspiration for the accent.
- **A familiar desktop-era visual language:** use square corners, compact typography, two-pixel bevels, inset surfaces, and a status-like footer. Borrow the structural character of the Windows 95 era while retaining an original palette and an ordinary responsive web document.
- **System fonts for now:** use Tahoma/Verdana with sans-serif fallbacks. A custom typeface is technically possible, but none was selected or added; the current design uses no downloaded font binaries or external font service.
- **Visible, accurate disclosures:** retain the explicit dual-licence label, clean-room development statement, local/no-telemetry statement, and brief release-readiness note. Do not promote full feature parity as an achieved fact while acceptance work remains open.

## Purpose and hierarchy

Introduce CurveBook as scientific software and help a visitor understand its central benefit and find the current release. Keep [`index.html`](../index.html) focused on three core topics: the product, sharing a complete workspace, and getting CurveBook, while [`privacy.html`](../privacy.html) provides the standalone per-distribution privacy policy (desktop installers, Windows MSIX / Microsoft Store, CLI/MCP, and the browser bundle). Use one primary action on the landing page, **Get CurveBook**, pointing to the download section. The compact navigation on `index.html` has **Share a workspace**, **Get CurveBook**, and **Privacy** (plus a footer link to `privacy.html`); `privacy.html` links back to `index.html` and its own policy sections (`#summary`, `#local-storage`, `#automation`, `#operators`, `#changes`). Avoid repeated calls to action, redundant return-to-top links, long resource menus, and technical tutorials on the landing page.

The product introduction describes the independently written Rust application and its biostatistics, nonlinear regression, and scientific graphing scope. MCP is a currently implemented access path, not the product's identity; keep its details in the setup guide. A brief release-status note preserves the distinction between implemented engine/MCP/CLI workflows and the unfinished graphical interface. Keep licence and clean-room information visible in the footer.

## Workspace sharing claim

Make **Share your full workspace. All at once.** a prominent feature within the opening document pane, with one native `.curvebook` file as the concrete mechanism. This is manual project-file sharing, not hosted or simultaneous collaboration.

The claim means the saved CurveBook project, not the computer's working directory or every application preference. `ProjectDocument` in [`crates/doc/src/project.rs`](../../crates/doc/src/project.rs) stores data, info/notes, analysis results/specifications, graphs, layouts, and checkpoints. `build_archive_entries` in [`crates/format/src/archive.rs`](../../crates/format/src/archive.rs) serializes the whole document and data-sheet CSV entries. See [`file-formats.md`](../file-formats.md). Linked external source files are not automatically attached; do not imply that unrelated files or runtime configuration are bundled. The recipient needs CurveBook to reopen the project.

## Claims and release honesty

Do not advertise **full feature parity** as achieved. [`STATUS.md`](../STATUS.md) and the [capability assessment](../target-app-parity.md) still record incomplete human GUI acceptance and release/platform QA. Broad implemented scientific scope does not establish complete product parity. Revisit the wording only when there is evidence for the entire claim, with an explicit scope and date. Avoid mutable feature, test, or model counts on the landing page.

The clean-room statement refers to developing CurveBook without using commercial statistical or graphing applications, inspecting their internals, or copying their proprietary code/assets, in accordance with [`AGENTS.md`](../../AGENTS.md). It is not a claim that no commercial editor, operating system, or development service has ever been used. Published scientific references and the project's independent implementation remain the basis of the methods.

The application is dual-licensed **MIT OR Apache-2.0**; retain the explicit **License:** label. Keep contributor and third-party records in their canonical repository files. The footer's source-code link provides the path to those records without another resource directory on the page.

## Visual language

The selected **Classic coral** direction replaces the initial turquoise desktop palette. Use warm-grey window chrome, white inset document surfaces, slate title bars, clearer two-pixel raised/inset borders, and a single coral primary action. Preserve the original pixel-art logo, including its small title-bar rendition and larger document wordmark. Keep the system sans-serif typography compact and readable. Do not add nonfunctional close/minimize controls, movable windows, a taskbar, desktop icons, or fake application interactions. The desktop styling supports an ordinary, accessible document.

| Role | Color |
|---|---|
| Desktop | `#ECEBE5` |
| Window chrome | `#D5D5CF` |
| Document surfaces | `#FAFAF6` |
| Title bars | `#29394D` |
| Text | `#222B35` |
| Secondary text | `#555D64` |
| Coral primary action | `#AB443D` |
| Darker coral links / action hover | `#8F3936` |
| Small project-file accent | `#1D6561` |

Concentrate the spectrum in the logo. Keep large surfaces neutral, headings in ink, and coral concentrated in the primary action and links. Use the darker coral for small text on grey chrome, and a dark teal only for the project-file label. The opening pane holds the introduction and sharing feature; its coral action jumps to the download section below. Retain every existing paragraph, heading, link destination, licence/clean-room disclosure, and the original logo bytes. Use system fonts; bundle no font files. The logo is an unchanged copy of `assets/app-icon/slack/curvebook.png`, with provenance recorded in [`ASSETS.md`](../../ASSETS.md).

## Behavior and maintenance

Use native fragment links, semantic headings, a skip link, keyboard focus on destination sections, and visible focus styling. Reflow at narrow widths rather than shrinking the page. Link to the full release list so Alpha prereleases remain discoverable; do not assume that `/releases/latest` or an unpublished asset exists.

Keep the site buildless: local HTML/CSS/PNG only, no scripts, external fonts, analytics, forms, or background requests across both [`index.html`](../index.html) and [`privacy.html`](../privacy.html). Preserve the content security and referrer policies. The application collects no telemetry; GitHub Pages itself logs IP addresses for security, as disclosed in the footer, [`privacy.html`](../privacy.html), and [GitHub documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages#data-collection).

At Alpha launch, update the pending-release wording only after verifying the published prerelease. GUI onboarding follows human GUI acceptance; a browser demo needs its own live browser QA. No changes to hosting or release state are part of preparing this page.
