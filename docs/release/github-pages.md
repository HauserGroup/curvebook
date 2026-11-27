# GitHub Pages Site: Design & Publication

The site consists of [`index.html`](../index.html), [`faq.html`](../faq.html), [`open-pzfx-files.html`](../open-pzfx-files.html), [`linux.html`](../linux.html), [`privacy.html`](../privacy.html), [`site.css`](../site.css), the attributed logo and workspace screenshots under [`images/`](../images/), committed `docs/404.html` and root redirect shims (`docs/get-started.html`, `docs/troubleshooting.html`), and the CI-generated manual and root metadata tree (`manual/index.html`, canonical `/manual/latest/`, active versioned snapshots under `manual/v<major>.<minor>/`, plus `_root/{sitemap.xml,llms.txt,llms-full.txt}` staged to the site root — built into a gitignored build directory such as `target/docs-manual/` by `cargo run -p xtask --features docs-site -- docs-site --build-manual` under `R12.10` / `R21.4`). `index.html` introduces CurveBook, showcases the desktop workspace, summarizes capabilities and Beta platform packages, and explains full-workspace file sharing; `faq.html` provides a Quick Facts table and structured answers to common questions; `open-pzfx-files.html` documents offline `.pzfx` and `.prism` table import; `linux.html` documents native Linux AppImage installation and headless CLI/MCP usage; and `privacy.html` publishes the dedicated plain-language privacy policy at `/privacy.html` for desktop installers, Windows MSIX / Microsoft Store, CLI/MCP, and browser-bundle users. The [design decisions](#design-decisions) below govern content and styling; [preview and checks](#preview-and-checks) and [publishing](#publish-alongside-beta) cover verification and release publication.

## Design decisions

Initial decisions: 2026-11-16–18. **Beta overhaul recorded: 2026-11-23.** **Flat readability overhaul recorded: 2026-11-26 (`R12.11`).** Scope: the public GitHub Pages landing pages ([`index.html`](../index.html), [`faq.html`](../faq.html), [`open-pzfx-files.html`](../open-pzfx-files.html), [`linux.html`](../linux.html), [`privacy.html`](../privacy.html)), fallback manual index (`manual/index.html`), shared stylesheet ([`site.css`](../site.css)), and generated static reference manual.

### Scope

This record governs the public CurveBook website and its GitHub Pages presentation: page structure, visitor-facing copy, visual styling, links, accessibility, and publication. It does not prescribe the desktop application interface, browser application interface, application themes, menus, dialogs, or scientific graph styles (governed separately by [`design-language.md`](../design-language.md)). References to windows, title bars, or desktop styling below describe historical website treatments only.

### Flat readability overhaul (2026-11-26, R12.11)

Retired the Classic coral beveled desktop window frame (`.desktop-window`, `.title-bar`, `.window-status`, `.window-statusbar`, 2px raised/recessed bevels, hard `3px 3px 0` drop shadows, and redundant `.document-brand` mastheads) in one coordinated pass across [`site.css`](../site.css), the five hand-written landing pages, and the generated static manual template (`xtask/src/docs_site/template.rs`):

- **Why the window frame was retired:** On dense scientific reference guides and multi-section manual pages, nested beveled window chrome, a dark title bar, a stacked two-row navigation strip, a verbose version strip, and a four-paragraph footer plus fake status bar competed visually with technical prose and equations while pushing headings below the fold. Switching to a calm flat two-surface layout preserves all zero-JS, strict-CSP, `no-referrer`, offline, and print guarantees while maximizing scannability and reading comfort.
- **Flat two-surface tokens and verified WCAG 2.2 contrast:** Replaced beveled grey chrome (`--chrome`, `--shadow`, `--deep`) with a warm stone backdrop (`--desktop`), crisp white reading canvas (`--paper`), faint subtle tint (`--surface-subtle`), 1px hairline borders (`--border`, `--border-strong`), dark blue WCAG AAA links (`--link`, `--link-hover`), and coral (`--accent`, `--accent-hover`) reserved strictly for the primary CTA button, hero accent, and warning/caution accents. Removed `-webkit-font-smoothing: antialiased` so native subpixel rendering stays crisp on non-Retina displays.

| Role | CSS token | Color | Verified WCAG 2.2 contrast |
|---|---|---|---|
| Page backdrop | `--desktop` | `#f5f4f0` | `--ink` on `--desktop`: `13.88:1` (AAA) |
| Content canvas | `--paper` | `#ffffff` | `--ink` on `--paper`: `15.27:1` (AAA) |
| Subtle surface (cards, callouts, TOC, code, zebra rows) | `--surface-subtle` | `#f7f6f2` | `--ink` on `--surface-subtle`: `14.12:1` (AAA) |
| Hairline borders | `--border` / `--border-strong` | `#dcd8d0` / `#c2bdb3` | Structural separation (paired with `2px solid var(--title)` focus outlines) |
| Primary text | `--ink` | `#1e2630` | `15.27:1` on `--paper`, `14.12:1` on `--surface-subtle` (AAA) |
| Headings & active navigation | `--title` | `#243752` | `12.05:1` on `--paper`, `10.95:1` on `--desktop` (AAA) |
| Secondary / muted text | `--muted` | `#525963` | `7.08:1` on `--paper` (AAA), `6.54:1` on `--surface-subtle`, `6.43:1` on `--desktop` (AA) |
| Links | `--link` | `#1d4f91` | `8.14:1` on `--paper`, `7.53:1` on `--surface-subtle`, `7.40:1` on `--desktop` (AAA) |
| Link hover | `--link-hover` | `#143a6d` | `11.32:1` on `--paper`, `10.28:1` on `--desktop` (AAA) |
| Coral primary CTA & hero accent | `--accent` | `#b83d37` | `--paper` on `--accent`: `5.58:1` (AA); `--accent` on `--paper`: `5.58:1` (AA) |
| Coral hover & warning titles | `--accent-hover` | `#962f2b` | `--paper` on `--accent-hover`: `7.67:1` (AAA); `--accent-hover` on `--surface-subtle`: `7.09:1` (AAA) |
| File-type & signed-status accent | `--file-ink` | `#156260` | `7.12:1` on `--paper` (AAA), `6.59:1` on `--surface-subtle` (AA) |

- **Single-row header & sticky manual sidebar:** Replaced the two-tier title bar + nav strip with `.site-shell` (`max-width: 1040px` on landing pages; `.site-shell.is-manual` `max-width: 1220px` on manual pages) and a single-row `.site-header` / `.header-inner` pairing a `32×32` `.wordmark` with six non-underlined `.site-nav` links (`aria-current="page"` indicated by bold `--title` text and a `2px` bottom rule) and, on manual snapshots, a quiet right-aligned `.header-utility` (`Home`) link, with the `.wordmark` also returning to the site home. At `>= 980px`, `.site-header` stays on one row and combined header + `.version-banner` height stays under `120px` at `1280px`. Manual pages pair a sticky `240px` left `.manual-sidebar` with a `minmax(0, 1fr)` main reading column in `.manual-layout`, collapsing cleanly to single-column flow below `980px`.
- **Compact single-block footer preserving required disclosures:** Replaced the four-paragraph footer plus fake `.window-statusbar` with one compact `.site-footer` separated by a top `1px solid var(--border)` hairline that states each required disclosure once: explicit `MIT OR Apache-2.0` license, one-sentence clean-room statement, zero-telemetry plus GitHub Pages IP-log notice, `Privacy Policy` link, `FAQ`, and `Source code & credits ↗`.
- **Unified reading measure & typography:** Enforced a `66ch` reading measure across prose paragraphs, lists, blockquotes, definition lists, `.callout`, and `.toc` blocks—including inside `<section>` and `<article>` wrappers on `faq.html`, `linux.html`, `open-pzfx-files.html`, and `privacy.html`—while letting `.table-wrap`, `<pre>`, `.doc-figure`, and card grids use the full column cleanly. Replaced `overflow-wrap: anywhere` on `.doc-body`, `th`, and `td` with `overflow-wrap: break-word` so wide tables scroll horizontally cleanly instead of breaking words mid-letter. Removed colliding `h2` bottom borders below `<hr>`, softened inline `<code>` to faint `--surface-subtle` with a `1px solid #e6e3dc` hairline, reserved monospace strictly for code/filenames/tabular figures (removing decorative monospace from `.pill`, `.platform-tag`, and `.badge`), and made whole `.section-card` tiles clickable via stretched title pseudo-elements.
- **Simplified top-placed download section (`index.html`, `site.css`):** Moved `#downloads` (**Get CurveBook**) directly after the hero `.intro` section (before screenshots, features, and workspace sharing), removed the duplicate hero download CTA so `.intro` flows straight into the single primary **Download from GitHub Releases ↗** button, and replaced technical packaging metadata with three plain-language desktop cards (macOS `.dmg`, Windows `.msi`, Linux AppImage), quiet secondary links (`Install guide`, `AI assistant (MCP) setup`, `Known issues`), and a single-line footnote pointing CLI, FreeBSD, and checksum details to `manual/latest/get-started.html`.
- **Static manual CSS, redirects & privacy sync:** `.doc-body img` scales attributed `1440×900` screenshots cleanly without horizontal overflow; pure-CSS `.math` / `.callout-*` / `<details class="toc">` rules render equations and callouts with zero JavaScript (`@media (forced-colors: active)` and `@media print` supported); root redirect shims (`docs/get-started.html`, `docs/troubleshooting.html`) and `docs/404.html` stay committed while generated manual HTML (`manual/index.html`, `manual/latest/`, `manual/v<major>.<minor>/`) and `_root/{sitemap.xml,llms.txt,llms-full.txt}` are built in CI into gitignored `target/docs-manual/` (`R12.10`/`R21.4`). `privacy.html` documents local storage paths, CRC-32 `v2` `ops.journal`, and GitHub Pages IP-log collection.

*(Superseded pre-`R12.11` Classic coral / Warm Stone beveled desktop-window design notes from Alpha and the 2026-11-23 Beta overhaul are preserved in git history.)*

### Workspace sharing claim

Make **Share your full workspace. All at once.** prominent, backed by `.curvebook` single-file archives, `.curvebook.d/` unpacked directories, and `.cbtemplate` templates. This is manual project-file sharing, not hosted or simultaneous live collaboration.

`ProjectDocument` in [`crates/doc/src/project.rs`](../../crates/doc/src/project.rs) stores data, info/notes, analysis results/specifications, graphs, layouts, and checkpoints; `build_archive_entries` in [`crates/format/src/archive.rs`](../../crates/format/src/archive.rs) serializes the whole document, data-sheet CSV entries, and embedded image assets (see [`file-formats.md`](../file-formats.md)). Linked external CSV/TSV files keep provenance paths and are not automatically attached outside the project; recipients need CurveBook to reopen the project.

### Claims and release honesty

Do not advertise **full feature parity** as achieved or embed mutable feature, test, command, or model counts in hand-written prose ([`pitfalls.md` §2](../pitfalls.md#2-hardcoded-headline-counts-going-stale)). Live engineering status belongs in [`STATUS.md`](../STATUS.md) and [`evidence/overview.md`](../evidence/overview.md).

The clean-room statement refers to developing CurveBook without using commercial statistical or graphing applications, inspecting their internals, or copying their proprietary code/assets, in accordance with [`AGENTS.md`](../../AGENTS.md). Retain the explicit **License:** (`MIT OR Apache-2.0`), clean-room disclosure, zero-telemetry + GitHub Pages IP-log privacy notice, and source code link in the footer. All visual assets under [`images/`](../images/) must carry verified attribution rows in [`ASSETS.md`](../../ASSETS.md).

All user-facing documentation on the site (landing pages, FAQ, manual reference pages, guides, and tutorials) should prefer `".pzfx files"` and `".prism files"` naming where appropriate rather than opaque euphemisms such as `"External Archive"` or `"XML Project"`. Lowercase `.prism` and `.pzfx` extensions (`.prism files`, `.pzfx files`, `*.prism`) are specifically allowed over clean-room and branding principles (`AGENTS.md` §1; enforced by `cargo xtask brands`), while bare or capitalised competitor product names, vendor documentation prose/screenshots, and vendor-generated sample files remain disallowed.

### Behavior and maintenance

Use native fragment links, semantic headings, a skip link, keyboard focus on destination sections (`tabindex="-1"`), native `<details>` disclosure for long TOCs, and visible `2px solid` focus outlines (`var(--title)`). Reflow cleanly down to `320px` with zero horizontal overflow. Link to the full GitHub Releases list rather than assuming `/releases/latest` (since prereleases do not populate `/releases/latest`).

Keep the site completely buildless to serve: local HTML/CSS/PNG only, zero scripts, zero external fonts, zero analytics, zero forms, and zero background requests across `index.html`, `privacy.html`, `manual/`, and root redirect shims. Preserve `Referrer-Policy: no-referrer` and the strict `Content-Security-Policy`, and keep the GitHub Pages IP-log disclosure accurate against [GitHub documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages#data-collection).

## Preview and checks

Generate a local preview of the full documentation manual (into the gitignored `target/docs-site/` directory) or build the deployable Pages manual tree (`index.html`, `latest/`, and active versioned snapshots into gitignored `target/docs-manual/`), and serve either `target/docs-site/` or a staged `target/pages-site/` with a local static HTTP server (`python3 -m http.server`). No external build toolchain, scripts, or external fonts are required:

```sh
cargo run -p xtask --features docs-site -- docs-site
cargo run -p xtask --features docs-site -- docs-site --check
cargo run -p xtask --features docs-site -- docs-site --build-manual target/docs-manual --check
```

At release tagging time, refresh screenshots, update the committed root redirect shims (`docs/get-started.html`, `docs/troubleshooting.html`) and `docs/404.html`, and verify the release manual and generated `_root/{sitemap.xml,llms.txt,llms-full.txt}` in `target/docs-manual/` (without committing generated HTML or SEO/LLM files under `docs/`):

```sh
cargo run -p xtask --features docs-site -- docs-site --release <version> --refresh-screenshots --check
```

### Search & AI discoverability metadata (`R12.8`, `R12.10`, `R21.4`)

- **Canonical `/manual/latest/` URLs:** `docs-site --build-manual` (and `--release <version>`) emits both the active versioned snapshot under `<manual-dir>/v<major>.<minor>/` and the unversioned canonical mirror at `<manual-dir>/latest/` inside a gitignored build directory (default `target/docs-manual/`), never committed under `docs/manual/`. Every page in `manual/latest/` carries a self-referencing `<link rel="canonical">` to `.../manual/latest/...`; versioned snapshot pages also point their canonical link to `/manual/latest/` and stay out of `sitemap.xml` so search engines and citations accumulate on stable URLs across releases. `manual/index.html`, root redirect shims (`docs/get-started.html`, `docs/troubleshooting.html`), `README.md`, and hand-written landing pages link to `manual/latest/`.
- **Sitemap (`sitemap.xml`):** Lists the hand-written landing pages (`LANDING_PAGES` in `xtask/src/pages_site.rs`: `index.html`, `open-pzfx-files.html`, `linux.html`, `faq.html`, `privacy.html`) and every generated manual page under `/manual/latest/`, and nothing else: redirect shims and versioned snapshots stay out so search engines index one copy of each page. It is emitted into `<manual-dir>/_root/sitemap.xml` by `docs-site --build-manual` (and `--sitemap`, which also refreshes `docs/404.html`) and staged to `_site/sitemap.xml` by `pages-stage`.
- **Open Graph & HTML Microdata (zero `<script>`):** All hand-written landing pages (`index.html`, `faq.html`, `open-pzfx-files.html`, `linux.html`, `privacy.html`) and generated manual pages carry Open Graph (`og:type`, `og:site_name`, `og:title`, `og:description`, `og:url`, `og:image`, `og:image:alt`) and `twitter:card="summary_large_image"` metadata pointing to the published base URL. Because the site enforces `script-src 'none'` (`default-src 'none'`), structured data uses HTML Microdata (`itemscope`, `itemtype`, `itemprop`) instead of `<script type="application/ld+json">`: `index.html` declares `schema.org/SoftwareApplication`, and `faq.html` declares both `schema.org/SoftwareApplication` (Quick Facts table) and `schema.org/FAQPage` (Question/Answer items).
- **LLM indexes (`llms.txt` and `llms-full.txt`):** `docs-site --build-manual` generates deterministic Markdown indexes into `<manual-dir>/_root/{llms.txt,llms-full.txt}` (staged to the site root by `pages-stage` and listed in `GENERATED_SITE_FILES` in `xtask/src/pages_site.rs`): `llms.txt` provides a concise structured directory of CurveBook's capabilities, platforms, formats, and canonical `/manual/latest/` links, and `llms-full.txt` bundles the full plain-text reference content (with internal doc links stripped) so AI assistants and retrieval tools can index CurveBook cleanly without scraping HTML chrome.
- **Custom domain (`CUSTOM_DOMAIN` in `pages_site.rs`):** `xtask/src/pages_site.rs` sets `CUSTOM_DOMAIN = Some("curvebook.org")` and `SITE_BASE_URL = "https://curvebook.org/"`, so `pages-stage` automatically emits `CNAME` (`curvebook.org`) and root `robots.txt` (`Sitemap: https://curvebook.org/sitemap.xml`), and `docs/404.html` resolves assets from `/`.
- **Verification & webmaster tools:** `docs-site --check` fails when generated `sitemap.xml`, `llms.txt`, or `llms-full.txt` is stale, when generated manual HTML or `GENERATED_SITE_FILES` are committed under `docs/`, or when a landing page lacks its exact canonical link or required Open Graph tags. Canonical links are the only absolute `<link href>` the checks accept, and only inside `SITE_BASE_URL`. With `CUSTOM_DOMAIN` active on `curvebook.org`, `pages-stage` publishes root `robots.txt` and `CNAME`, and Google Search Console / Bing Webmaster Tools can verify the domain directly via DNS `TXT` record (or an HTML file added to `COMMITTED_SITE_FILES` in `xtask/src/pages_site.rs`).

### What Pages publishes (`R12.9`)

Pages serves only the files on the publish list in `xtask/src/pages_site.rs`, not the whole `docs/` folder, so the internal Markdown kept under `docs/` (roadmap, planning, evidence, reference sources, schemas) is never served next to the site and generated manual HTML and SEO/LLM files stay out of git (`R12.10`, `R21.4`). The list combines committed files under `docs/`—landing pages (`LANDING_PAGES`), root redirect shims (`get-started.html`, `troubleshooting.html`), `COMMITTED_SITE_FILES` (`404.html`, `site.css`), and everything under `images/`—with the CI-generated outputs under `target/docs-manual/` (`manual/index.html`, `manual/latest/`, active versioned snapshots under `manual/v<major>.<minor>/` excluding `manifest.json`, and `GENERATED_SITE_FILES` `_root/{sitemap.xml,llms.txt,llms-full.txt}`, plus generated `CNAME` and `robots.txt` when `CUSTOM_DOMAIN` is set). `404.html` is generated with `site_base_path()`-prefixed asset and navigation URLs so it renders cleanly at any nested missing path. Markdown and symlinks are never published.

`.github/workflows/pages.yml` runs on pushes to `main` that touch `docs/`, `xtask/`, `crates/`, `apps/curvebook-cli/src/cli_catalog.rs`, `Cargo.toml`, or the workflow/setup itself (and on demand): it builds and verifies the manual HTML into `target/docs-manual`, stages only the publish list into `_site`, and deploys that folder. Its `deploy` job is the only place outside `release.yml` that may hold `pages: write` and `id-token: write` (`cargo xtask telemetry`). To build, inspect, or stage the publish list locally:

```sh
cargo run -p xtask --features docs-site -- docs-site --build-manual target/docs-manual --check
cargo xtask pages-stage --manual-dir target/docs-manual --list
cargo xtask pages-stage --manual-dir target/docs-manual --check
cargo xtask pages-stage --manual-dir target/docs-manual target/pages-site
```

(When `target/docs-manual/` is already populated by `docs-site --check` or `docs-site --build-manual`, `--manual-dir` may be omitted.) `--check` (also part of `docs-site --check`) fails when a published page links to, or redirects to, a local file that is not published, or when `sitemap.xml` names one. A new top-level page or file needs an entry in the list; otherwise its links fail this check and it is not deployed. The repository stays public, so this keeps internal files off the website, not out of view.

**One-time switch from branch publishing (maintainer, admin access).** Until the source is switched, `pages.yml` runs but its `deploy` job cannot publish, so switch right after the workflow lands on `main`:

1. **Settings → Pages → Build and deployment → Source:** choose **GitHub Actions**. GitHub creates the `github-pages` environment, limited to `main`.
2. **Actions → Pages → Run workflow** on `main`, and wait for the `deploy` job.
3. Check that `/`, `/faq.html`, `/open-pzfx-files.html`, `/privacy.html`, `/sitemap.xml`, `/llms.txt`, `/llms-full.txt`, `/manual/`, `/manual/latest/get-started.html`, and the redirect pages return `200`, and that internal files such as `/ROADMAP.md`, `/pitfalls.md`, `/reference/concepts.md`, and `/schemas/preferences.schema.json` return `404`.
4. Remove `docs/.nojekyll`, which only branch publishing reads, and record the check in [`history/release/pages-verification.md`](../history/release/pages-verification.md). Search engines drop the removed URLs over the following weeks; once the site is verified in Search Console, its Removals tool can speed that up.

Relative asset and page paths (`site.css`, `images/*.png`, `privacy.html`, `manual/index.html`, `index.html`) support deployment below `/curvebook/` or at a domain root. Verify cross-page links (`index.html` ↔ `privacy.html` ↔ `manual/index.html` and generated manual internal `#fragment` links), keyboard focus, WCAG AA text contrast, `@media (forced-colors: active)`, `@media print`, zero external browser network requests, and responsive (`1280px` / `850px` / `700px` / `390px` / `320px`) layouts without horizontal overflow, then run the full gate suite:

```sh
cargo xtask screenshots --check
cargo xtask assets
cargo xtask brands
cargo xtask cleanroom
cargo xtask secrets
cargo xtask telemetry
cargo run -p xtask --features docs-site -- docs-site --check
cargo xtask pages-stage --check
cargo xtask status
git diff --check
```

## Publish alongside Beta

Keep hosting strictly limited to the staged publish list ([What Pages publishes](#what-pages-publishes-r129)). The Actions source deploys the staged folder as is, without Jekyll.

1. Cut and verify the Beta prerelease using [`releasing.md`](releasing.md#alpha-and-beta-releases-with-the-release-workflow). Confirm the release contains `curvebook-<version>-macos-universal.dmg`, `curvebook-<version>-windows-{x64,arm64}.msi`, `curvebook-<version>-linux-{x86_64,aarch64}.AppImage`, `curvebook-<version>-freebsd-x86_64.tar.gz`, `SHA256SUMS.txt`, and `THIRD-PARTY-LICENSES.html` with passing Sigstore attestations.
2. Run `cargo run -p xtask --features docs-site -- docs-site --release <version> --refresh-screenshots --check` from the tagged commit so the 6 clean-room workspace/specimen PNGs, `docs/404.html`, and the root redirect shims (`docs/get-started.html`, `docs/troubleshooting.html`) are refreshed and the generated manual plus `_root/{sitemap.xml,llms.txt,llms-full.txt}` in `target/docs-manual/` are verified against the release tag SHA (without committing generated outputs under `docs/`), and confirm `#downloads` links in [`index.html`](../index.html) point to `manual/latest/get-started.html`, `manual/latest/troubleshooting.html#known-issues`, and `manual/latest/mcp-for-beginners.html`.
3. Run the local responsive (`1280px` / `850px` / `700px` / `390px` / `320px`), keyboard, contrast, `forced-colors: active`, print, zero-network-request, and `cargo xtask` checks listed in [Preview and checks](#preview-and-checks).
4. Ensure **Settings → Pages → Source** is **GitHub Actions**, so `.github/workflows/pages.yml` publishes the staged site on pushes to `main`.
5. Wait for GitHub Pages deployment to succeed, then verify `https://curvebook.org/`, `https://www.curvebook.org/` (redirect to apex), `/robots.txt`, `/faq.html`, `/open-pzfx-files.html`, `/privacy.html`, `/404.html`, `/llms.txt`, `/llms-full.txt`, `/sitemap.xml`, `/manual/`, `/manual/latest/get-started.html`, `/manual/latest/troubleshooting.html#known-issues`, and the root redirect shims (`/get-started.html`, `/troubleshooting.html`) live in a browser across desktop and narrow mobile widths, and record the verification in [`history/release/pages-verification.md`](../history/release/pages-verification.md).

The documentation site is separate from the `1.0`-deferred WASM application under `packaging/web/`. Its server-header files are not a GitHub Pages configuration, and publishing `/docs` does not publish `curvebook-web`.

## Local verification

Dated verification records for the initial landing page, the Classic coral restyle, and the privacy page are archived in [`history/release/pages-verification.md`](../history/release/pages-verification.md).
