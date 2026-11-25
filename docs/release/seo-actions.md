# Search discoverability: possible actions

This is a catalogue of actions that could help people find CurveBook through search engines. It lists options, not commitments. Claimed work lives in [`ROADMAP.md`](../ROADMAP.md) (search work so far: [`R12.8`](../ROADMAP.md#r128-search-discoverability-for-the-pages-site) and [`R12.9`](../ROADMAP.md#r129-publish-only-the-site-on-pages)); before starting an action here, add or claim a roadmap item for it. How the site is built, checked, and published is described in [`github-pages.md`](github-pages.md).

Recorded 2026-11-24. Statuses below go stale; check the roadmap before relying on them.

## Ground rules

Every action must fit the project rules that already apply to the site:

- **Neutral wording.** Pages may name the `.pzfx` and `.prism` file formats by their lowercase extensions. Every other reference to the commercial application that writes those files fails `cargo xtask brands` and `cargo xtask cleanroom`, unless a separate, scoped policy decision allows it (see [Needs a policy decision](#needs-a-policy-decision)).
- **Only claims we can check.** Each factual claim on a page is backed by a test or a command a reader can run, as `apps/curvebook-cli/tests/r12_8_pzfx_landing_page.rs` does for [`open-pzfx-files.html`](../open-pzfx-files.html). Desktop GUI steps need a passing human-path test (see `R12.7`).
- **Static, script-free, no external requests.** The site sends no analytics and loads nothing from other origins. Measurement must come from the search engines' own webmaster tools, which need nothing on the page except a verification file or meta tag.
- **Generated, not hand-maintained.** Anything that can drift (sitemap, canonical links, metadata on manual pages) belongs in `cargo xtask docs-site` and is checked by `--check`.
- **No gaming.** No hidden text, keyword lists, doorway pages, or repeated competitor names. These risk penalties and look like trading on another product's reputation.

## Who we want to reach

| Search intent | Example searches | Asset that answers it |
|---|---|---|
| **Open or convert a file** | "open pzfx without license", "pzfx to csv", ".prism file viewer" | File-format pages |
| **Find a free or open alternative** | "free alternative to <the commercial app>", "biostatistics software linux", "open source curve fitting" | Comparison page, platform pages, listings on alternative-software sites |
| **Do an analysis** | "EC50 dose response fit", "two-way ANOVA Dunnett", "Kaplan-Meier curve" | Task tutorials (`R12.2`) |
| **Check trust** | "offline statistics software GDPR", "validated curve fitting NIST" | Privacy page, verification page |

Navigational searches for the commercial product itself ("<product> download", "<product> login") are not a target. We would not rank, and trying looks like bad faith.

Search volumes above are unmeasured. Check them in a keyword tool or Search Console before committing writing time to a group.

## Status key

| Status | Meaning |
|---|---|
| **Done** | Merged to `main` |
| **Branch** | Implemented on an unmerged branch |
| **Ready** | Possible now under current rules; nobody has claimed it |
| **Blocked** | Waits for other roadmap work |
| **Decision** | Needs a maintainer or policy decision first |
| **Account** | Needs a maintainer to sign in to an outside service; cannot be automated |

## 1. Technical foundations

| Action | Status | Notes |
|---|---|---|
| Generated `sitemap.xml` of landing pages and the newest manual snapshot | Done (`R12.8`, CI-only `R21.4`) | Emitted into `target/docs-manual/_root/sitemap.xml` by `docs-site --build-manual`; checked by `--check` |
| One canonical link on each hand-written landing page | Done (`R12.8`) | The checker only allows canonical links inside the Pages site |
| Publish only the site, not the whole `docs/` folder | Done (`R12.9`) | Stops internal Markdown (roadmap, planning, evidence) from being indexed next to the site |
| Stable `/manual/latest/` URLs with canonical links from versioned snapshots | Done (`R12.10`) | Concentrates ranking on one URL per page instead of splitting it across `v0.1`, `v0.2`, … |
| Open Graph and summary-card tags on landing and manual pages | Done (`R12.10`) | Improves link previews when people share the site; not a ranking factor |
| `SoftwareApplication` structured data (HTML Microdata) on the home and FAQ pages | Done (`R12.10`) | Uses `itemscope`/`itemtype` Microdata rather than JSON-LD to keep zero `<script>` tags and `STRICT_CSP`. Helps search engines and AI parsers identify the page as software; rich results require ratings/reviews, so expect no visible snippet change |
| `FAQPage` structured data (HTML Microdata) on `faq.html` + facts table | Done (`R12.10`) | Self-contained first-sentence answers and a structured facts table give AI tools quotable facts. Google restricts FAQ rich results to a small set of government/health sites, so expect no rich snippet |
| `llms.txt` and `llms-full.txt` | Done (`R12.10`, CI-only `R21.4`) | Read by some AI assistants and crawlers; generated into `target/docs-manual/_root/` during CI builds and checked by `docs-site --check` with internal doc links stripped |
| Align `README.md` opening paragraph with canonical site summary | Branch (`R12.10`) | AI training corpora and coding agents read GitHub READMEs directly; the first paragraph now states free/open-source, OSes, offline/zero-telemetry, `.pzfx`/`.prism`, and GUI/CLI/MCP |
| Custom domain (for example `curvebook.org`) | Decision | Separates CurveBook from `hausergroup.github.io`, enables root-level `robots.txt` and `llms.txt`, and avoids migrating Search Console/backlinks later. `CUSTOM_DOMAIN` in `xtask/src/pages_site.rs` (`R12.10`) makes cutover a single-config switch that auto-emits `CNAME` and `robots.txt` |
| A `404.html` page that links back to the home page and manual | Ready | Pages serves it for missing URLs under the project path once the site is deployed through Actions (`R12.9`). Keeps visitors from dead ends after URLs change |
| Accurate `<lastmod>` in the sitemap | Decision | The sitemap leaves it out to stay deterministic. A value from the last git commit touching each source page would be deterministic and is used by search engines when it is accurate |
| `BreadcrumbList` structured data on manual pages | Ready | Generated from the existing section structure. Can show the page path in results |
| Per-page `<meta name="description">` written for search wording | Ready | Generate from each reference page's front matter in `cargo xtask docs-site`; `--check` fails if one is missing or duplicated |
| Image weight and dimensions on landing pages | Ready | Keep explicit `width`/`height`, `loading="lazy"` below the fold, and compressed screenshots. Keep the browser app's `.wasm` off landing pages |
| `hreflang` alternates | Blocked | Only relevant once translated pages are published |
| A `robots.txt` for the site | Decision | Pages serves `robots.txt` only at the organisation root site, which is a different repository. `R12.9` removes the main reason to need one |

## 2. Measurement

| Action | Status | Notes |
|---|---|---|
| Verify the site in Google Search Console and submit the sitemap | Account (`R12.8`) | Verification by an HTML file in the site or a meta tag on the home page; no DNS access needed |
| Verify the site in Bing Webmaster Tools | Account | Bing can import a verified Search Console property. Bing also feeds several other search engines and some AI assistants |
| Measure search volume per intent group before writing pages | Ready | Use a keyword tool or, after launch, the Search Console query report |
| Review impressions and position per intent group after each new page | Account | If a group does not move, improve those pages before adding more |
| Monthly manual check in AI search tools | Ready | Because the site sends no analytics, run a fixed query set (e.g. "free alternative for .pzfx files", "open source dose-response fitting on Linux") across ChatGPT, Claude, Perplexity, and Google AI Overviews and log whether CurveBook is cited accurately |
| IndexNow pings on deploy | Decision | Lets Bing and others pick up changed pages quickly. The ping is an outbound request from the Pages workflow, so it needs a `cargo xtask telemetry` decision even though shipped binaries are unaffected |

## 3. Content

### File formats (strongest opening)

| Action | Status | Notes |
|---|---|---|
| Open `.pzfx` and `.prism` files and export tables to CSV (CLI route) | Done (`R12.8`) | [`open-pzfx-files.html`](../open-pzfx-files.html), checked by a CLI test |
| Add the desktop route to the same page (File › Open, Add to current project, CSV export) | Blocked | Needs the `R10.29` GUI work and `R10.30` human-path tests |
| Separate pages per search phrasing (".pzfx to CSV", ".prism to CSV", "what is kept and what is lost") | Ready | Only if Search Console shows the single page ranking for some phrasings and not others. Avoid near-duplicate pages |
| View a `.pzfx` file in the browser, with nothing uploaded | Blocked | Needs a hosted browser app (`R11`). GitHub Pages cannot send the security headers the browser app requires |
| A tutorial: `.prism` file to CSV | Blocked | First guide of the `R12.2` pilot; starts after Beta |

### Alternative and platform searches

| Action | Status | Notes |
|---|---|---|
| A Linux page: install the AppImage, open files, run analyses | Ready | Linux AppImages ship in the Beta download set. Few strong pages answer "biostatistics / curve fitting on Linux" |
| An offline and data-protection page aimed at clinical and regulated labs | Ready | Builds on [`privacy.html`](../privacy.html): no network access, no telemetry, files never leave the machine |
| A verification page: how results are checked against NIST StRD and other published references | Ready | Draw on `docs/evidence/` and the `D1` oracle tests. A distinctive trust signal for scientists. Avoid hard-coded counts; link to generated reports instead |
| A neutral feature & statistical methods matrix | Ready | Captures comparison intent safely without naming the commercial application (no `brands` exemption needed): platforms, formats read/written, analysis families, curve-fitting methods, exports, and automation interfaces |
| A comparison page against the commercial application | Decision | Needs a scoped `brands` exemption and a legal review. See below |
| A terminology map for people switching from the commercial application | Decision | Same exemption. Must be written from public knowledge and CurveBook's own format parser, never from that product's documentation (clean-room rules) |

### Analysis tasks (largest volume, slowest to win)

| Action | Status | Notes |
|---|---|---|
| Task tutorials: ANOVA with export, dose-response EC50, Kaplan–Meier, publication figure | Blocked | `R12.2` pilot, after Beta. Each GUI step needs a passing human-path test |
| More tutorials drawn from the GUI workflow list | Blocked | Only after the pilot shows which intent groups gain impressions |
| Link every tutorial to the file-format page and the home page | Ready (with each tutorial) | Internal links pass ranking from strong pages to new ones |
| Generic method explainers ("what is a four-parameter logistic fit") | Decision | Competes with university and textbook pages; only worth it if it links straight into a CurveBook tutorial |

## 4. Off-site listings

Only listings whose pages already rank for our target searches count as search work here. Outreach that builds reputation rather than rankings is listed separately below.

| Action | Status | Notes |
|---|---|---|
| Listings on AlternativeTo, SaaSHub, and Slant, as an alternative to the commercial application | Account | These sites' pages already rank for "alternative to …" searches, so a listing puts CurveBook on the results page immediately. The listing text names the other product on their site, not ours |
| Listing in [bio.tools](https://bio.tools) (`biotoolsID: curvebook`) | Account | ELIXIR's life-science software registry indexes statistical, curve-fitting, and graphing tools with EDAM ontology terms and feeds downstream directories. Register through the Web UI once public release and manual URLs are live, pick a single SPDX licence (`MIT` or `Apache-2.0`; note the dual licence and Tier 2 FreeBSD CLI in the description), and add a second maintainer as editor. Automating updates from `release.yml` via the bio.tools API would be an outbound CI request and needs a `cargo xtask telemetry` decision |
| GitHub repository description, social preview image, and topics (`pzfx`, `dose-response`, `ec50`, `survival-analysis`, `curve-fitting`, `biostatistics`, `graphing`, `statistics`, `rust`, `mcp`) | Decision / Account | A public change to repository settings; GitHub topic pages rank and feed GitHub search and AI tools, and uploading `docs/images/workspace-split-fit.png` (or a `1280×640` crop) gives repo links a proper card in chat/email |
| MCP registry listings (official MCP Registry, Smithery, Glama, mcp.so, `awesome-mcp-servers`) | Decision / Account | CurveBook's unique discovery channel for AI agents. Requires a `server.json` and a low-friction install path (e.g. Homebrew, `cargo-binstall`, `.mcpb`, or container) since `curvebook-cli mcp` currently ships as a GitHub Release binary. Put broad discovery keywords in `server.json` / registry metadata rather than bloating individual MCP tool schemas |
| Wikidata item and consistent one-line identity | Blocked | Helps language models disambiguate "CurveBook" (always paired with "open-source biostatistics") and merge facts across GitHub, bio.tools, AlternativeTo, and the website; add `sameAs` once live. Needs independent third-party references (bio.tools, AlternativeTo, Zenodo/JOSS) first to satisfy notability/verifiability |
| A Flathub listing for "linux" searches | Decision | Flatpak packaging was dropped from the Beta download set (`R0.28`). Reopening it is a packaging decision, not just a listing |

**Outreach, not search work:** `CITATION.cff` + Zenodo DOI and a JOSS paper (creates Google Scholar / Crossref citations and academic backlinks), Homebrew, AUR, and winget packages, and community discussions (Show HN, `r/bioinformatics`, `r/labrats`, `r/biostatistics`, `r/rust`, Mastodon/Bluesky). These build citations, trust, and installs, are frequently retrieved by AI search tools, and provide the independent references Wikidata requires, but belong in a separate outreach plan.

## Needs a policy decision

Two actions in this catalogue conflict with current rules and must not start without a recorded decision:

1. **Naming the commercial application on our pages.** A comparison page or terminology map needs that name in its title and headings to rank. It would need a `brands`/`cleanroom` exemption scoped to those pages only, versioned and dated comparisons limited to verifiable facts (price, platforms, licence, file compatibility), a disclaimer of non-affiliation at the top, and a legal review before publishing.
2. **Outbound requests from the Pages workflow** (IndexNow). These touch the zero-egress policy in `AGENTS.md`, even though shipped binaries stay offline.

## Things not to do

- Use the other product's name or logo in CurveBook's identity, domain, or taglines (for example "<product>-compatible").
- Claim "identical results" or "drop-in replacement" unless round-trip and oracle tests prove it for the specific case.
- Use screenshots of the other product's interface.
- Repeat the other product's name across pages, or hide keywords in text, alt attributes, or metadata.
- Add an analytics script, tracking pixel, or third-party font or widget to measure results.
- Bid on the other product's name in paid search ads without legal review; ad text must never suggest affiliation.

## Suggested order

1. Finish what is in flight: merge `R12.10`, complete the `R12.9` switch-over, verify Search Console and Bing, submit the sitemap.
2. Cheap technical items: `404.html`, generated meta descriptions, breadcrumbs, sitemap `lastmod` decision.
3. Listings on alternative-software sites and bio.tools, and the GitHub topics decision.
4. Linux, offline/data-protection, and verification pages.
5. Decide on the comparison page and terminology map.
6. After Beta: the `R12.2` tutorial pilot, then more tutorials where Search Console shows demand.
