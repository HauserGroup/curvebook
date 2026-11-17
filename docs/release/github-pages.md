# GitHub Pages publication

The site is [`index.html`](../index.html), [`site.css`](../site.css), and the attributed existing logo at [`images/curvebook.png`](../images/curvebook.png). It is one static page introducing the product, full-workspace file sharing, and downloads. See [`website-design.md`](website-design.md) for the current design and claim decisions.

## Preview and checks

Serve `docs/` with a local static HTTP server. No build, package installation, scripts, or external fonts are required. Relative asset paths support deployment below `/curvebook/` or at a domain root. Verify fragment destinations, keyboard focus, text contrast, and desktop/narrow layouts, then run the relevant documentation/asset policies:

```sh
cargo xtask assets
cargo xtask brands
cargo xtask cleanroom
cargo xtask secrets
cargo xtask telemetry
cargo xtask status
git diff --check
```

## Publish alongside Alpha

Keep hosting disabled during preparation. No deployment workflow or extra token permissions are needed.

1. Review the final source and site against `R0.20`, `R0.21`, and the final renamed MCP/setup checks. Include the site/logo in the branding and asset-provenance review.
2. Review and publish the Alpha prerelease using [`releasing.md`](releasing.md#alpha-releases-with-the-existing-builder). Confirm that its public release page contains the intended packages and checksums. The full releases list includes prereleases; `/releases/latest` does not.
3. Update the page's pre-alpha/Alpha-in-preparation wording to match the actual release. Preserve the GUI limitation and the distinction between saved-project sharing and live collaboration.
4. When the reviewed site is authorized to land and launch, select **Settings → Pages → Deploy from a branch → main → /docs**. The empty `.nojekyll` disables Jekyll processing. Branch publication serves the complete `docs/` directory; canonical Markdown links intentionally point to GitHub for rendering.
5. Wait for GitHub's Pages deployment to succeed. Use the URL shown in settings, then verify HTTPS, logo/styles, section buttons, external release/setup/status links, and a narrow viewport. Only after that add the live URL to repository entry points and record hosted verification.

GitHub documents [branch/folder publishing](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site) and [entry files and `.nojekyll`](https://docs.github.com/en/pages/getting-started-with-github-pages/creating-a-github-pages-site). Once enabled, pushes to the chosen source branch update the public site. This preparation does not authorize merging the website branch into `main` or enabling hosting.

The documentation site is separate from the WASM application under `packaging/web/`. Its server-header files are not a GitHub Pages configuration, and preparing this page does not publish the browser application.

## Local verification

Verified on 2026-11-17 on `docs/R0.14-pages-site`, based on `2a948b0` plus the website, icon-attribution, design/publication notes, and scoped roadmap changes. The policy runner was built from this isolated checkout with `CARGO_NET_OFFLINE=true` and a stable `CARGO_TARGET_DIR=/tmp/cb-pages-branch`. `cargo xtask assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, and `status` all passed, as did `git diff --check`. No Rust code, dependencies, generated command/schema files, workflows, or fixtures changed; full workspace CI was not rerun for this static documentation change.

The HTML audit checked IDs, fragment destinations, relative assets, canonical guide paths, documentation links, the unchanged logo bytes, the licence/clean-room statements, and absence of scripts/forms/external assets. Default normal-text color pairs meet AA contrast, with a minimum measured ratio of `4.63:1`. Browser review covered desktop at 1280px, narrow layouts at 390px and 320px without horizontal overflow, workspace/download navigation, destination focus, the skip link, and an empty warning/error console. These checks establish the documentation site's local behavior, not a hosted deployment or full graphical-application readiness.

### Classic coral restyle

Verified on 2026-11-17 against `27f7017` plus the site markup/styles, design notes, and scoped roadmap/publication-note changes. The existing paragraphs, headings, complete set of link destinations, and logo bytes were preserved. The logo now also appears in the document wordmark, using the same attributed PNG. Normal-text color pairs meet AA contrast (minimum `4.54:1`); coral button text measures `5.79:1`, and a darker coral is used for small links on grey chrome.

Browser review covered 1280px desktop and 390px/320px narrow layouts, with no horizontal overflow. Workspace/download fragment links, keyboard destination focus, the visible dotted focus indicator, and the skip-to-main link passed; the warning/error console was empty. Using the same isolated checkout, offline configuration, and stable build directory, `assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, and `status` and whitespace checks passed. No application code, dependencies, assets, schemas, command registries, workflows, or hosting settings changed, so full workspace CI was not rerun for the restyle.
