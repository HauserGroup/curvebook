# GitHub Pages for the MCP Alpha

The small public site lives in [`docs/index.html`](../index.html), with [`site.css`](../site.css) and an existing CurveBook icon copied to [`images/curvebook.png`](../images/curvebook.png). It covers MCP onboarding, current status, and downloads. Setup instructions, detailed status, commands, and release procedures remain in their canonical Markdown documents on GitHub; the site links to them rather than maintaining a second manual.

The selected turquoise desktop design uses a pale mint backdrop, warm white surfaces, petrol title bars, and deep teal actions. Square corners, thin bevels, and inset panels provide the desktop character, using the existing pixel-art logo and system fonts. It remains one page: the main navigation and raised action buttons jump to setup, status, downloads, guides/help, or privacy sections; those sections contain the external documentation and release links. Title bars provide a return-to-top link, and fragment destinations accept keyboard focus.

Prepare the site now and publish alongside the Alpha. `R10` is not a dependency for MCP onboarding. After `R10.13` human-GUI acceptance, expand the site with verified GUI setup and walkthroughs. A browser demo also needs the live browser QA recorded under `R8.5`; do not infer readiness from a successful WASM build or the semantic agent harness.

## Preview

Open `docs/index.html` in a browser, or serve `docs/` with a local static HTTP server. No build, package installation, JavaScript, or external fonts are required. All local URLs are relative so the page works below `/curvebook/` as well as at a domain root. The main guides intentionally open on GitHub, where Markdown is rendered.

Before landing, check local assets and anchor links, desktop and narrow layouts, keyboard navigation, and the relevant repository policies:

```sh
cargo xtask assets
cargo xtask brands
cargo xtask cleanroom
cargo xtask secrets
cargo xtask telemetry
cargo xtask status
```

## Publish alongside Alpha

Keep Pages disabled while preparing this change. Adding these files does not enable hosting or publish an Alpha release. No custom deployment workflow or additional token permissions are needed.

1. Land the reviewed files on `main`. Complete `R0.20`, `R0.21`, and the final renamed MCP/beginner setup checks on the Alpha revision. Include this page and its icon in the launch branding/provenance review.
2. Review and publish the Alpha prerelease and its assets using the [existing release procedure](releasing.md#alpha-releases-with-the-existing-builder). Confirm that the release list is publicly accessible, packages and checksums are present, and download/setup links resolve for a signed-out visitor.
3. Update the site’s pre-alpha label, Alpha status paragraph, and review date to reflect the release actually published. Keep the GUI limitation visible. Link directly to the reviewed Alpha tag from the download section if desired; retain the full release-list link because `/releases/latest` excludes prereleases. Do not guess a future tag or asset filename.
4. In **Settings → Pages → Build and deployment**, select **Deploy from a branch**, **main**, and **/docs**, then save. The empty `docs/.nojekyll` disables Jekyll processing. This publishes the complete `docs/` directory as static files; the friendly entry point is `index.html` and canonical Markdown links still point to GitHub.
5. Wait for GitHub’s Pages build/deployment to succeed. Open the URL shown in Pages settings (expected project path: `https://hausergroup.github.io/curvebook/`), check HTTPS and the stylesheet/icon, follow the onboarding, status, and download links, and check a narrow viewport and keyboard focus. Only then add the live URL to the repository description and README and record the hosted result in the roadmap.

GitHub documents [branch/folder publishing](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site) and [the entry file and `.nojekyll` behavior](https://docs.github.com/en/pages/getting-started-with-github-pages/creating-a-github-pages-site). GitHub runs the Pages deployment itself even with branch publishing; no hand-written workflow is added here. Once enabled, pushes to `main` update the published `docs/` contents. Review site changes accordingly.

## Upkeep

- Keep the beginner guide as the setup source of truth and `STATUS.md` / `ROADMAP.md` as the evidence records. Reconcile the short site summary and review date when release readiness changes. Avoid mutable feature/test counts in site prose.
- Keep downloads pointed at the release list or a verified published Alpha tag. Draft releases and Actions artefacts are not public downloads. Follow the release notes for supported platforms, signing, and known issues.
- Keep the site static, with local styles and image assets and system fonts. The HTML policy blocks scripts and background connections; ordinary user-followed links lead to GitHub and the setup guides. No data is submitted through the page.
- CurveBook’s no-telemetry policy applies to the software. GitHub Pages itself logs visitor IP addresses for security, as disclosed in the site’s privacy note and [GitHub’s documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages#data-collection). The AI application’s own privacy settings still govern chat content and tool results.
- This documentation site is separate from the WASM application packaged by `packaging/web/`. The app’s CSP/COOP/COEP server-header configuration remains in that package; GitHub Pages does not apply its `_headers` or `.htaccess` files. Do not publish the browser application here as part of this preparation.

## Local preparation evidence

Verified on 2026-11-16 against base revision `2a948b0` plus the uncommitted Pages files and associated documentation/asset entries. The policy runner was built offline from the working source with `CARGO_TARGET_DIR=/tmp/cb-pages`, then reused unchanged for `assets`, `brands`, `cleanroom`, `secrets`, `telemetry`, and `status`; all passed. `git diff --check` and a local HTML/link audit passed, including relative assets, section IDs, canonical Markdown paths/anchors, icon byte identity, and absence of scripts/forms/external assets. Browser review covered the desktop layout, narrow layouts at 390px and 320px without horizontal overflow, local anchor navigation, keyboard focus and the skip-to-main link, loaded logo/style, and an empty warning/error console. These are documentation-site checks; they do not establish desktop/WASM application readiness or a hosted Pages deployment. Full workspace CI was not rerun for this static documentation-only change.

The subsequent turquoise desktop styling pass on the same base revision preserved every existing heading, paragraph, status description, and the logo bytes. The HTML audit verified section IDs, canonical guide paths/anchors, and that all main navigation links and raised action buttons target this page. Default normal-text color pairs meet AA contrast (minimum measured ratio `4.63:1`). Browser checks covered 1280px desktop and 390px/320px narrow layouts; all main section jumps, the return-to-setup button, the first-analysis jump, home/skip-link keyboard focus, and absence of horizontal overflow or console warnings/errors passed. The same scoped policy checks and `git diff --check` were rerun for the final uncommitted site/docs changes; no application, workflow, or publishing settings were changed.
