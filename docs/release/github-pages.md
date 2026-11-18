# GitHub Pages publication

The site consists of [`index.html`](../index.html), [`privacy.html`](../privacy.html), [`site.css`](../site.css), the attributed existing logo at [`images/curvebook.png`](../images/curvebook.png), and the static documentation manual entry at [`manual/index.html`](../manual/index.html) (with frozen release snapshots written under `docs/manual/v<major>.<minor>/` by `cargo xtask docs-site --release <version>`). `index.html` introduces the product, full-workspace file sharing, and downloads, while `privacy.html` publishes the dedicated plain-language privacy policy at the stable `/privacy.html` path for desktop installers, Windows MSIX / Microsoft Store, CLI/MCP, and browser-bundle users. See [`website-design.md`](website-design.md) for the current design and claim decisions.

## Preview and checks

Generate a local preview of the full documentation manual (into the gitignored `target/docs-site/` directory) and serve either `docs/` or `target/docs-site/` with a local static HTTP server. No external build toolchain, scripts, or external fonts are required:

```sh
cargo xtask docs-site
cargo xtask docs-site --check
```

At release tagging time, generate a frozen versioned snapshot under `docs/manual/v<major>.<minor>/`:

```sh
cargo xtask docs-site --release <version>
```

Relative asset and page paths (`site.css`, `images/curvebook.png`, `privacy.html`, `manual/index.html`, `index.html`) support deployment below `/curvebook/` or at a domain root. Verify cross-page links (`index.html` ↔ `privacy.html` ↔ `manual/index.html` and generated manual internal `#fragment` links), keyboard focus, text contrast, and desktop/narrow (`1280px` / `390px` / `320px`) layouts without horizontal overflow, then run the full gate suite:

```sh
cargo xtask assets
cargo xtask brands
cargo xtask cleanroom
cargo xtask secrets
cargo xtask telemetry
cargo xtask docs-site --check
cargo xtask status
git diff --check
```

## Publish alongside Alpha

Keep hosting disabled during preparation. No deployment workflow or extra token permissions are needed.

1. Review the final source and site against `R0.20`, `R0.21`, and the final renamed MCP/setup checks. Include the site/logo in the branding and asset-provenance review.
2. Review and publish the Alpha prerelease using [`releasing.md`](releasing.md#alpha-releases-with-the-existing-builder). Confirm that its public release page contains the intended packages and checksums. The full releases list includes prereleases; `/releases/latest` does not.
3. Update the page's pre-alpha/Alpha-in-preparation wording to match the actual release. Preserve the GUI limitation and the distinction between saved-project sharing and live collaboration.
4. When the reviewed site is authorized to land and launch, select **Settings → Pages → Deploy from a branch → main → /docs**. The empty `.nojekyll` disables Jekyll processing. Branch publication serves the complete `docs/` directory; canonical Markdown links intentionally point to GitHub for rendering.
5. Wait for GitHub's Pages deployment to succeed. Use the URL shown in settings, then verify HTTPS, logo/styles, section buttons, `/privacy.html`, external release/setup/status/security links, and a narrow viewport. Only after that add the live URL to repository entry points and record hosted verification.

GitHub documents [branch/folder publishing](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site) and [entry files and `.nojekyll`](https://docs.github.com/en/pages/getting-started-with-github-pages/creating-a-github-pages-site). Once enabled, pushes to the chosen source branch update the public site. This preparation does not authorize merging the website branch into `main` or enabling hosting.

The documentation site is separate from the WASM application under `packaging/web/`. Its server-header files are not a GitHub Pages configuration, and preparing this page does not publish the browser application.

## Local verification

Verified on 2026-11-17 on `docs/R0.14-pages-site`, based on `2a948b0` plus the website, icon-attribution, design/publication notes, and scoped roadmap changes. The policy runner was built from this isolated checkout with `CARGO_NET_OFFLINE=true` and a stable `CARGO_TARGET_DIR=/tmp/cb-pages-branch`. `cargo xtask assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, and `status` all passed, as did `git diff --check`. No Rust code, dependencies, generated command/schema files, workflows, or fixtures changed; full workspace CI was not rerun for this static documentation change.

The HTML audit checked IDs, fragment destinations, relative assets, canonical guide paths, documentation links, the unchanged logo bytes, the licence/clean-room statements, and absence of scripts/forms/external assets. Default normal-text color pairs meet AA contrast, with a minimum measured ratio of `4.63:1`. Browser review covered desktop at 1280px, narrow layouts at 390px and 320px without horizontal overflow, workspace/download navigation, destination focus, the skip link, and an empty warning/error console. These checks establish the documentation site's local behavior, not a hosted deployment or full graphical-application readiness.

### Classic coral restyle

Verified on 2026-11-17 against `27f7017` plus the site markup/styles, design notes, and scoped roadmap/publication-note changes. The existing paragraphs, headings, complete set of link destinations, and logo bytes were preserved. The logo now also appears in the document wordmark, using the same attributed PNG. Normal-text color pairs meet AA contrast (minimum `4.54:1`); coral button text measures `5.79:1`, and a darker coral is used for small links on grey chrome.

Browser review covered 1280px desktop and 390px/320px narrow layouts, with no horizontal overflow. Workspace/download fragment links, keyboard destination focus, the visible dotted focus indicator, and the skip-to-main link passed; the warning/error console was empty. Using the same isolated checkout, offline configuration, and stable build directory, `assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, and `status` and whitespace checks passed. No application code, dependencies, assets, schemas, command registries, workflows, or hosting settings changed, so full workspace CI was not rerun for the restyle.

### Privacy policy page (`R8.9`)

Verified on 2026-11-18 on `docs/R8.9-privacy-policy` (`1bccc83` base): [`privacy.html`](../privacy.html) reuses [`site.css`](../site.css) and the attributed [`images/curvebook.png`](../images/curvebook.png) without adding scripts, forms, inline styles, font binaries, or external requests, and [`index.html`](../index.html) links to `privacy.html` from its navigation and privacy footer. Static HTML/Markdown audits verified strict `Content-Security-Policy` and `Referrer-Policy` metadata, semantic heading order, unique IDs, cross-page relative paths, `#fragment` targets (`tabindex="-1"`), AA color-pair contrast (minimum `4.54:1`), narrow `390px`/`320px` word-wrap bounds (`overflow-wrap: anywhere` on `<code>`), `git diff --check`, and `cargo xtask` (`assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, `status`).
