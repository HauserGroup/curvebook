# GitHub Pages Verification Records (Archive)

> **Archive.** Dated local verification records moved out of [`release/github-pages.md`](../../release/github-pages.md) by `R0.31`. They describe the site as it was on the stated dates.

## Landing page (2026-11-17)

Verified on 2026-11-17 on `docs/R0.14-pages-site`, based on `2a948b0` plus the website, icon-attribution, design/publication notes, and scoped roadmap changes. The policy runner was built from this isolated checkout with `CARGO_NET_OFFLINE=true` and a stable `CARGO_TARGET_DIR=/tmp/cb-pages-branch`. `cargo xtask assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, and `status` all passed, as did `git diff --check`. No Rust code, dependencies, generated command/schema files, workflows, or fixtures changed; full workspace CI was not rerun for this static documentation change.

The HTML audit checked IDs, fragment destinations, relative assets, canonical guide paths, documentation links, the unchanged logo bytes, the licence/clean-room statements, and absence of scripts/forms/external assets. Default normal-text color pairs meet AA contrast, with a minimum measured ratio of `4.63:1`. Browser review covered desktop at 1280px, narrow layouts at 390px and 320px without horizontal overflow, workspace/download navigation, destination focus, the skip link, and an empty warning/error console. These checks establish the documentation site's local behavior, not a hosted deployment or full graphical-application readiness.

## Classic coral restyle

Verified on 2026-11-17 against `27f7017` plus the site markup/styles, design notes, and scoped roadmap/publication-note changes. The existing paragraphs, headings, complete set of link destinations, and logo bytes were preserved. The logo now also appears in the document wordmark, using the same attributed PNG. Normal-text color pairs meet AA contrast (minimum `4.54:1`); coral button text measures `5.79:1`, and a darker coral is used for small links on grey chrome.

Browser review covered 1280px desktop and 390px/320px narrow layouts, with no horizontal overflow. Workspace/download fragment links, keyboard destination focus, the visible dotted focus indicator, and the skip-to-main link passed; the warning/error console was empty. Using the same isolated checkout, offline configuration, and stable build directory, `assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, and `status` and whitespace checks passed. No application code, dependencies, assets, schemas, command registries, workflows, or hosting settings changed, so full workspace CI was not rerun for the restyle.

## Privacy policy page (`R8.9`)

Verified on 2026-11-18 on `docs/R8.9-privacy-policy` (`1bccc83` base): [`privacy.html`](../../privacy.html) reuses [`site.css`](../../site.css) and the attributed [`images/curvebook.png`](../../images/curvebook.png) without adding scripts, forms, inline styles, font binaries, or external requests, and [`index.html`](../../index.html) links to `privacy.html` from its navigation and privacy footer. Static HTML/Markdown audits verified strict `Content-Security-Policy` and `Referrer-Policy` metadata, semantic heading order, unique IDs, cross-page relative paths, `#fragment` targets (`tabindex="-1"`), AA color-pair contrast (minimum `4.54:1`), narrow `390px`/`320px` word-wrap bounds (`overflow-wrap: anywhere` on `<code>`), `git diff --check`, and `cargo xtask` (`assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, `status`).
