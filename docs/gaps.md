# Gaps

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** Wave 1–3, Slice 0, Risk Spikes `S1`/`S2` (both **GO**), headless Alpha/Core workflows, `apps/lumencraft` CLI/`--control` (`6 passed`), Tier-1 localization (`G13`), and Beta library/engine wiring (`G7`–`G12`, `G14`) landed and verified; remaining estimates narrowed to live OS windowing/`wgpu`/`wasm32` hosting, 96-curve 4PL batch optimization (`16.91s` → `<50ms`), remaining library edge items, and `1.0` hardening · **Target:** GraphPad Prism 11 (11.1.x)

Every shortfall between LumenCraft's current workspace and GraphPad Prism 11.1, **ranked by how much it blocks a working scientist from switching**. Unless the user assigns a specific task, pick the highest-ranked unblocked gap below that falls in the current step of the [execution order](roadmap.md#execution-order) and inside the scope of the next gate, read its linked implementation guide first, implement and test it end to end, and update this file, [`target-app-parity.md`](target-app-parity.md), the domain parity checklist, and [`ROADMAP.md`](../ROADMAP.md) in the same PR.

Gap IDs (`G1`–`G14`) are permanent; ranks and hour estimates adjust as work lands. **Alpha** marks work required to pass the [Alpha gate](roadmap.md#alpha-gate) (Essentials scope); **Core** marks work added by the [Core-workflow gate](roadmap.md#core-workflow-gate); **Beta** marks work required for beta. A gap split across gates shows the hours for each part, and its details state which scope is Alpha. Work the next gate does not need waits, however high the gap ranks. All landing follows the [landing rules](roadmap.md#landing-rules).

**The Hours column here is the source of truth.** The Alpha-gate table and milestone table in [`roadmap.md`](roadmap.md) and the headline numbers in [`ROADMAP.md`](../ROADMAP.md) are sums of it; change them together.

## Ranked

| Rank | Gap | Hours | By gate | Implementation guide & checklist |
|---:|---|---:|:---:|---|
| 1 | [G1 Numerical foundations & NIST StRD oracle](#g1-numerical-foundations-and-nist-strd-oracle) | 1–2 | Core (external `D1` edge fixtures) | [statistics-implementation.md](statistics-implementation.md) |
| 2 | [G2 Typed data tables, reactive DAG & `.lumencraft` / CSV I/O](#g2-typed-data-tables-reactive-dag-and-lumencraft--csv-io) | 1–2 | Core (crash journal & forward migrations) | [data-tables-and-transforms.md](data-tables-and-transforms.md), [file-format-parity.md](file-format-parity.md) |
| 3 | [G3 Core biostatistics, ANOVA/REML, effect sizes & multiplicity](#g3-core-biostatistics-anovareml-effect-sizes-and-multiplicity) | 3–5 | Core (secondary options & `D1` edge fixtures) | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 4 | [G4 Nonlinear regression solver, profile CIs & built-in equations](#g4-nonlinear-regression-solver-profile-cis-and-built-in-equations) | 4–7 | Core (log band grid & 5 binding equations) | [curve-fitting-implementation.md](curve-fitting-implementation.md), [fitting-parity.md](fitting-parity.md) |
| 5 | [G5 Publication chart compiler, broken/log axes & SVG/PDF export](#g5-publication-chart-compiler-brokenlog-axes-and-svgpdf-export) | 3–5 | Core (`additional_ticks`, PDF 1.7 TTF, viewer check) | [graphing-and-layouts.md](graphing-and-layouts.md), [graphing-parity.md](graphing-parity.md) |
| 6 | [G6 Desktop & WASM shell: Navigator, subcolumn grid & inspectors](#g6-desktop-and-wasm-shell-navigator-subcolumn-grid-and-inspectors) | 25–40 | Alpha 15–25 (live OS/`wgpu`/`wasm32` host) · Core 10–15 | [ui-parity.md](ui-parity.md) |
| 7 | [G7 Prism 11 Multiple Variables workflows, Calculated Variables & Multifactor ANOVA](#g7-prism-11-multiple-variables-workflows-calculated-variables-and-multifactor-anova) | 2–4 | Beta (MV grouping *t*-test / KM & multi-factor split) | [data-tables-and-transforms.md](data-tables-and-transforms.md), [statistics-implementation.md](statistics-implementation.md) |
| 8 | [G8 Survival (Kaplan–Meier, Cox PH), contingency CIs, ROC, GLMs & PCA](#g8-survival-kaplanmeier-cox-ph-contingency-cis-roc-glms-and-pca) | 3–5 | Beta (standalone Bland–Altman tab, Barnard/MI/Passing–Bablok) | [statistics-implementation.md](statistics-implementation.md), [stats-parity.md](stats-parity.md) |
| 9 | [G9 Custom equation DSL, implicit/ODE models & derived transforms](#g9-custom-equation-dsl-implicitode-models-and-derived-transforms) | 2–4 | Beta (`InitialValues`/transform blocks, `<C>`+ scoping) | [curve-fitting-implementation.md](curve-fitting-implementation.md), [data-tables-and-transforms.md](data-tables-and-transforms.md) |
| 10 | [G10 Clean-room `.prism` & `.pzfx` project import](#g10-clean-room-prism-and-pzfx-project-import) | 2–3 | Beta (extended edge fixtures & UI drag-drop) | [file-format-parity.md](file-format-parity.md) |
| 11 | [G11 Multi-panel Page Layouts, style cloning & print/TIFF pipeline](#g11-multi-panel-page-layouts-style-cloning-and-printtiff-pipeline) | 4–6 | Beta (live OS clipboard/print APIs & canvas snap) | [graphing-and-layouts.md](graphing-and-layouts.md), [hardware-parity.md](hardware-parity.md) |
| 12 | [G12 Performance budgets, large-table virtualization & fuzzing](#g12-performance-budgets-large-table-virtualization-and-fuzzing) | 3–5 | Beta (optimize 96-curve 4PL `16.91s` → `<50ms`) | [development.md](development.md) |
| 13 | [G13 Localization & regional scientific number formatting](#g13-localization-and-regional-scientific-number-formatting) | 8–12 | 1.0 (Tier-2 locales, RTL `ar`/`he`, OS CJK font verify) | [localization-parity.md](localization-parity.md) |
| 14 | [G14 Headless batch CLI, `.lctemplate` pipelines & full MCP catalog](#g14-headless-batch-cli-lctemplate-pipelines-and-full-mcp-catalog) | 0 | ✅ verified (`cli` 7, `mcp` 25, `lumencraft` 6, `web` 1; GUI window flags with `G6`) | [scripting-and-automation.md](scripting-and-automation.md), [mcp.md](mcp.md) |

**Remaining totals (one agent):** Alpha `15–25 h` (headless Alpha already passes via CLI/MCP/`UiApp`; live OS desktop/`wgpu`/`wasm32` hosting `G6`) · Core workflows add `22–36 h` (`G1`–`G5`, `G6` Core UI polish), `37–61 h` cumulative · Beta adds `16–27 h` (`G7`–`G12`), `53–88 h` cumulative · `G13` adds `8–12 h`, `61–100 h` for all gaps. Milestone `M7` carries a further `12–18 h` of cross-OS GPU/trackpad/installer hardening not tracked as a parity gap, giving **`73–118 h`** to `1.0`. These are implementation hours and exclude human review.

## Known defects

Found in code review of commits `9b29c08`–`8f2ba04` (2026-11-13) and not yet fixed. They are not counted in the Hours column above. Each row names its gap and the failure to cover with a regression test.

| Gap | Location | Defect | Failure to test |
|---|---|---|---|
| `G8` | `crates/stats/src/categorical.rs` — `koopman_solve_lower`/`_upper`/`_lower_c_zero`/`_upper_a_zero` | When bracketing fails or `brent_root` returns `Err`, each solver falls back to `.unwrap_or(endpoint)` and reports that endpoint as a finite Koopman RR confidence limit. | A table where 120 halvings/doublings never change the score's sign, or the score is non-finite at an endpoint, yields a bound near `rr·2^±120` that does not solve the score equation. Expected: an open bound or `None`, never a non-root. |
| `G11` | `crates/engine/src/cmd_sci.rs` — `resolve_raster_target`; `crates/render/src/rasterizer.rs` — `RasterImage::new` | Explicit `width_px`/`height_px` are capped only per axis (`MAX_RASTER_DIM_PX = 16384`), with no total-pixel budget, and the buffer is allocated with `Vec::with_capacity`, which aborts on allocation failure. | PNG/TIFF export at `16384×16384` requests a 1 GiB buffer and aborts on `wasm32` or a low-memory host instead of returning `RenderError`. Add a pixel budget and use `try_reserve`. |
| `G2` | `crates/engine/src/cmd_data.rs` — external delimited-file link creation (`read_to_string(path).unwrap_or_default()`) | Read errors on the linked file are discarded, and delimiter and decimal-separator detection run on empty text. | Linking a missing, unreadable or non-UTF-8 file (for example semicolon-delimited with decimal commas) succeeds with default separators; later refreshes parse it wrongly. Expected: the link command reports the read error. |
| `G12` | `crates/engine/src/lib.rs` — `Session::execute`, `undo_stack` | Every command, read-only ones included, deep-clones the whole `ProjectDocument`, and every mutating snapshot is kept on an undo stack with no limit. | With a 10M-cell table, each `table.exportCsv` copies the whole project, and N edits keep N full copies until the process runs out of memory. Skip the clone for read-only commands and cap undo depth or memory. |
| `G9` | `crates/fit/src/dsl.rs` — `Parser::parse_add`/`parse_mul`/`parse_cmp`, `enum Expr` | `MAX_PARSE_DEPTH` limits parentheses, calls, unary chains and `^`, but not left-associative binary chains, and `Expr` still derives recursive `Clone`/`Debug`. Stack safety depends on every walker being written iteratively. | `Y = X+X+…` with about 200k terms builds a left-deep tree. Any recursive `clone()`, `{:?}` or new recursive pass overflows the stack, and `wasm32` aborts. Cap total AST node count or depth in the parser. |
| `G10` | `crates/format/src/pzfx.rs` — `estimate_table_cells` vs `build_table_archetype` | The import cell budget repeats the builder's per-kind sizing logic instead of measuring what the builder allocates. | If the builder changes (for example Nested pads groups to `max_units`, or Survival gains a subcolumn), the `MAX_IMPORT_CELLS` check undercounts without anyone noticing. Derive the count from the builder or the built table. |

---

## Details by gap

### G1 Numerical foundations and NIST StRD oracle
**Crates:** `crates/math`, `crates/testkit`, `xtask` · **Spec:** [`statistics-implementation.md` §1](statistics-implementation.md#1-numerical-foundations-cratesmath)
- **Done:**
  - `crates/math` (`L0`) & `crates/testkit`: Neumaier/two-pass moments, cancellation-free special functions & noncentral/Dunnett/range distributions, pivoted QR + Björck refinement, SVD, Cholesky, `SymEigen`, quadrature, Brent root-finding, `DeterministicRng`, 106-bit `DoubleDouble` oracle, and 58-dataset NIST StRD corpus verifier (`cargo xtask corpus`).
  - NIST StRD & `D1` wiring: `crates/stats` (`nist_strd` `4 passed`, `d1_reference_oracles` `20 passed`) and `crates/fit` (`nist_strd` `1 passed`, `d1_nist_and_pharma_oracles` `18 passed`, including `Eckerle4` and `Rat43` inside `slice0_vertical_gate`).
- **Missing (`1–2 h`):**
  - Expanded external `D1` R/SciPy edge-case reference fixtures across unbalanced three-way Type III SS, missing-data REML, tied-event Cox PH, and cluster validation.

### G2 Typed data tables, reactive DAG and `.lumencraft` / CSV I/O
**Crates:** `crates/palette`, `crates/doc`, `crates/format`, `crates/engine` · **Spec:** [`data-tables-and-transforms.md`](data-tables-and-transforms.md)
- **Done:**
  - `crates/palette` (`L0`), `crates/doc` (`L1`): all 8 `TableArchetype`s, 8 `SubcolumnMode`s + `SummarizedPoint`, non-destructive exclusions, `FloatingNote`, `InfoSheet`, `LinkedDataRegion` + 4-stage `ImportLinkSpec`, `ProjectDocument` upstream DAG extraction.
  - `crates/format` (`L3`, `36 passed` including `18` `corrupt_archive_fuzz` and `3` `prop_format`): lossless `.lumencraft` ZIP+JSON+CSV save/reopen, RFC-4180 CSV/TSV import/export, schema migration (`migrate_document_json` from `v0` -> `CURRENT_SCHEMA_VERSION`), and cycle rejection (`detect_dependency_cycles`).
  - `crates/engine` (`L5`, `48 passed`): topological recalculation DAG (`DataSheet -> LinkedDataRegion / Transform / Analysis -> ResultSheet -> Graph -> Layout`), live cross-table `Paste Link` & reactive transpose, atomic undo/redo command journal, verified end-to-end in `slice0_vertical_gate` and `e2e_workflows` (`11 passed`).
- **Missing (`1–2 h`):**
  - Desktop/WASM crash-recovery lock/autosave journal (atomic lockfile / Web Locks API) and multi-version forward migration adapters beyond baseline `v0 -> current`.

### G3 Core biostatistics, ANOVA/REML, effect sizes and multiplicity
**Crates:** `crates/stats` · **Spec:** [`statistics-implementation.md` §2–§4](statistics-implementation.md#2-descriptive-statistics-normality--outlier-detection)
- **Done:**
  - `crates/stats` (`36 passed`, `20` `d1_reference_oracles`, `4` `nist_strd`, `6` `prop_stats`) wired end-to-end through `engine`/`cli`/`mcp`/`UiApp`: descriptive stats, normality/lognormality battery (D'Agostino–Pearson $K^2$, Shapiro–Wilk $W$, Anderson–Darling $A^{*2}$, KS), Grubbs & ROUT outliers; unpaired/Welch/paired/ratio-paired/lognormal/one-sample *t*-tests + Cohen's $d$, Hedges' $g$, Glass's $\Delta$ (noncentral-$t$ CIs); Mann–Whitney, Wilcoxon, two-sample KS (Hodges–Lehmann CIs); ordinary/Welch/RM one-way ANOVA (GG $\hat{\varepsilon}$), Kruskal–Wallis, Friedman, two-way & three-way ANOVA (Type I/III SS) + $\eta^2, \eta_p^2, f$, REML mixed model; multiplicity (Tukey, Dunnett, Šídák, Bonferroni, Holm–Šídák, Dunnett T3, Games–Howell, Dunn, linear trend, BH/BY/BKY FDR) + Analysis Checklists.
- **Missing (`3–5 h`):**
  - `NewmanKeuls` and `FisherLsd` in `MultiplicityProcedure` / `engine`, `multiple_row_t_tests` command wiring, standalone `qq_expected_quantiles` result tab exposure, and expanded `D1` R/SciPy edge-case reference fixtures.

### G4 Nonlinear regression solver, profile CIs and built-in equations
**Crates:** `crates/fit` · **Spec:** [`curve-fitting-implementation.md` §1–§3](curve-fitting-implementation.md#1-nonlinear-solver-architecture-cratesfit)
- **Done:**
  - `crates/fit` (`55 passed`, `18` `d1_nist_and_pharma_oracles`, `16` `dsl_fuzz_props`, `1` `nist_strd`, `4` `prop_fit`) wired end-to-end: Moré QR trust-region LM + Nelder–Mead warm start, differential weights ($1/Y, 1/Y^2, 1/X, 1/\text{SD}^2$, Poisson), Cauchy robust & ROUT, box constraints, constant hooks, shared global parameters, Venzon–Moolgavkar profile-likelihood CIs (`CiBound::Open`), confidence/prediction bands, standard-curve `InterpolatedUnknowns` tab, extra-sum-of-squares *F*-test, AICc + Akaike weights, replicate/runs/homoscedasticity/AD/KS diagnostics + `Residuals` tab (studentized & Q-Q residuals), Dose–Response/Enzyme Kinetics/Exponentials/Lines/Peaks/Growth/Radiation equations verified in `Slice 0` and `e2e_workflows` (`11 passed`).
- **Missing (`4–7 h`):**
  - Adaptive/log-grid confidence band sampling (`build_curve_overlay` currently samples 101 linear-`x` points), conditional `"Don't fit points when..."` / `"Plot a function"` exclusion rules, centered polynomial forms & specialized semi-log/log-log/probit lines, and 5 missing receptor-binding/allosteric equations (`TwoSiteBinding`, `TotalAndNonspecificBinding`, `SaturationSpecificWithNonspecific`, `KineticAssociationDissociation`, `AllostericModulator`).

### G5 Publication chart compiler, broken/log axes and SVG/PDF export
**Crates:** `crates/chart`, `crates/text`, `crates/palette`, `crates/render`, `crates/svg`, `crates/pdf` · **Spec:** [`graphing-and-layouts.md`](graphing-and-layouts.md)
- **Done:**
  - **Risk Spike `S2` = GO** (`73 passed` default / `32 passed` `--no-default-features` w/ empty `BUNDLED_FONTS`; `0.293–0.395 ms` PDF generation vs `0.422 ms` default; selectable Core-14 Helvetica/Symbol Greek/math text + outlined fallback, `FlateDecode`, `MediaBox`/`TrimBox`/`BleedBox`, structural parser checks).
  - Deterministic `GraphSpec -> ChartScene` compiler for XY, Column (bar+beeswarm, box, violin, half-violin), Grouped, Contingency/Parts-of-Whole, FloatingBar, Volcano, Bland–Altman, ROC, Forest, and Heatmap; all 7 `AxisScale`s + 1–3 broken axis segments; solid/dashed error bars; auto-stacked live pairwise *p*/*q* brackets; SVG (`crates/svg`), PDF 1.4+ (`crates/pdf`), and 72–1200 DPI PNG/TIFF (`crates/render`) wired through `engine`/`cli`/`mcp`/`UiApp`.
- **Missing (`3–5 h`):**
  - Standalone `additional_ticks` exposure on custom `AxisSpec`, `PDF 1.7` metadata upgrade + TrueType glyph subset embedding (beyond Core-14 Type1 + outlines), and manual visual inspection of exported PDF/SVG in two independent GUI viewers (`S2` validated programmatically).

### G6 Desktop and WASM shell: Navigator, subcolumn grid and inspectors
**Crates:** `crates/ui-egui`, `crates/tools`, `apps/lumencraft`, `apps/lumencraft-web` · **Spec:** [`ui-parity.md`](ui-parity.md)
- **Done:**
  - **Risk Spike `S1` = GO** at engine/UI-shell layer (`lumencraft-tools` `15 passed`, `1M`-cell `120` frames `p50=0.0390ms`, `p95=0.0432ms`, `max=0.0612ms`, tail-jump `0.0150ms`; `lumencraft-ui-egui` `41 passed`, `1M`-cell `p50=0.061ms`, `p95=0.507ms`, `max=7.795ms`, `Select All + exclude` `0.044ms`; `wasm32` sysroot unavailable locally).
  - Pure-Rust `UiApp` state shell + software RGBA/PNG frame compositor (`crates/ui-egui`): 5-folder Project Navigator, Family pane, Welcome/`Clone Pipeline` dialog, two-tier virtualized subcolumn grid with exclusions & `Import & Paste Special`/`Paste Link`, Analyze dialog (`Cmd/Ctrl+T`, `Change Data Analyzed`, duplicate-name disambiguation), multi-tab Result viewer + Interpret checklist, `Change Graph Type` gallery, Format Graph/Axes/Pairwise-Comparisons inspectors, WCAG AA themes, plus `apps/lumencraft` (`6 passed`) CLI/`--headless-once`/`--control` wiring and `apps/lumencraft-web::init_web_app()` (`1 passed`).
- **Missing (`25–40 h`: Alpha `15–25 h` · Core `10–15 h`):**
  - Live OS desktop windowing / `eframe`/`wgpu` render loop hosting `UiApp` (currently software compositor + headless CLI/`--control`), live `wasm32` WebGPU/WebGL2 browser build & verification, interactive pointer drag-reorder & inline editing on Navigator/Info/`FloatingNote`/layout canvas, `F6` gallery toggle, canvas ruler bars, and customizable keyboard shortcut mapping.

### G7 Prism 11 Multiple Variables workflows, Calculated Variables and Multifactor ANOVA
**Crates:** `crates/doc`, `crates/transform`, `crates/stats`, `crates/fit`, `crates/chart`, `crates/ui-egui` · **Spec:** [`data-tables-and-transforms.md` §2.3 & §4](data-tables-and-transforms.md#23-multiplevariables-role-inference-bulk-recoding--categorical-level-ordering)
- **Done:** In-table `=` Calculated Variables engine (`100k` rows in `12.023 ms`), column role inference/locking, bulk recoding & `LevelOrderSpec`, Multifactor ($N$-way Type I/III) ANOVA, BubbleScatter / ConvexHull / ConfidenceEllipse / CategoricalSummary plots, and `UiApp` MV Variable Role / Formula / Level Order wiring (`e2e_workflows` passing).
- **Missing (`2–4 h`):** `MultipleVariables` grouping-column selector directly inside two-group *t*-test (`TwoGroupSpec`), Cartesian multi-factor splitting in `lumencraft-fit` (`extract_from_multivar`), and `MultipleVariables` Kaplan–Meier analysis wiring.

### G8 Survival (Kaplan–Meier, Cox PH), contingency CIs, ROC, GLMs and PCA
**Crates:** `crates/stats`, `crates/chart`, `crates/engine` · **Spec:** [`statistics-implementation.md` §5](statistics-implementation.md#5-contingency-survival-roc--multivariate-analysis)
- **Done:** Kaplan–Meier + log-rank/Gehan–Breslow–Wilcoxon/trend + Cox PH (Breslow/Efron ties, Schoenfeld residuals) + KM staircase graph (censor ticks, CI bands, number-at-risk table); Contingency ($\chi^2$, Fisher exact, Baptista–Pike OR, Koopman RR, Wilson CIs, $\phi$, Cramér's $V$); ROC (Hanley–McNeil / DeLong AUC CIs); Multiple linear/logistic/Poisson GLMs; PCA (Horn parallel analysis) + K-means / hierarchical clustering, wired via `engine` & tested in `e2e_workflows`.
- **Missing (`3–5 h`):** Standalone `AnalysisKind::BlandAltman` result-sheet computation, Barnard's exact test, Passing–Bablok regression comparison details, Multiple Imputation, and expanded `D1` R/SciPy Cox PH / GLM / PCA edge fixtures.

### G9 Custom equation DSL, implicit/ODE models and derived transforms
**Crates:** `crates/fit`, `crates/transform` · **Spec:** [`curve-fitting-implementation.md` §4](curve-fitting-implementation.md#4-user-defined-equation-language-cratesfitdsl)
- **Done:** Safe bytecode VM for custom equations (`dsl_fuzz_props` `16 passed`, `IF`/`AND`/`OR`, `<A>`/`<B>`, `brent_root` implicit equations, RK45 ODE integration) and all derived transforms (`Transform`, `Normalize`, `Baseline`, `Transpose`, `Prune`, `Simulate`, `Smooth/Differentiate/AUC`), wired through `engine` & `e2e_workflows`.
- **Missing (`2–4 h`):** `[InitialValues]` / parameter-transform multi-section DSL blocks and generalized dataset-scoped `<C>`/`<D>`/`<N>` prefixes (`dsl.rs` currently scopes `<A>`/`<B>`).

### G10 Clean-room `.prism` and `.pzfx` project import
**Crates:** `crates/format` · **Spec:** [`file-format-parity.md`](file-format-parity.md)
- **Done:** Clean-room importers for `.prism` (ZIP/JSON/CSV) and `.pzfx` (XML tables, subcolumns, titles, exclusions, Info constants) with bounded decompression/XML depth (`corrupt_archive_fuzz` `18 passed`, `interop_and_export` `4 passed`), wired through `engine`/`cli`/`UiApp`.
- **Missing (`2–3 h`):** Additional clean-room synthetic nested/survival/MV `.prism`/`.pzfx` edge-case fixtures and interactive UI drag-drop import validation.

### G11 Multi-panel Page Layouts, style cloning and print/TIFF pipeline
**Crates:** `crates/chart`, `crates/render`, `crates/pdf`, `crates/ui-egui` · **Spec:** [`graphing-and-layouts.md` §5–§6](graphing-and-layouts.md#5-consistent-styling-wand--clone-style--palettes)
- **Done:** Multi-graph `LayoutSheet` composer with plot-origin alignment & auto-lettering (`A`, `B`, `C`), `graph.applyStyleFrom` ("Wand"), uncompressed/Deflate/LZW TIFF export, headless `ClipboardBundle` (PDF+SVG+HTML+PNG+locale TSV), and calibrated physical-scale `PrintJobSpec` PDF spool payload (`/TrimBox`/`/BleedBox`).
- **Missing (`4–6 h`):** Live OS pasteboard integration (`NSPasteboard`/Win32/Wayland/Browser), OS print spooler submission (`NSPrintOperation`/Win32/CUPS/`window.print()`), and interactive layout WYSIWYG drag-snap polish.

### G12 Performance budgets, large-table virtualization and fuzzing
**Crates:** `crates/engine`, `crates/format`, `crates/mcp`, `crates/ui-egui`, `apps/lumencraft-cli` · **Spec:** [`development.md`](development.md#fuzzing)
- **Done:** `lumencraft-cli perf --json` harness, Spike `S1` `1M`-cell grid virtualization (`0.0390 ms` tools / `0.061 ms` UI p50 `< 16.67 ms`), `100k`-row CalcVar (`12.023 ms` `< 25 ms`), archive round-trip (`93.338 ms`), 300 DPI SVG+PDF+PNG (`241.83 ms`), and property/fuzz harnesses (`corrupt_archive_fuzz` `18`, `dsl_fuzz_props` `16`, `command_sweep_props` `4` across all `73` commands, `protocol_props` `11`, `prop_format` `3`, `prop_stats` `6`, `prop_fit` `4`).
- **Missing (`3–5 h`):** Optimize 96-curve 4PL batch fit from measured `16,914.673 ms` (`16.91 s`) down to the `< 50 ms` budget target, plus nightly `cargo-fuzz` corpus runs.

### G13 Localization and regional scientific number formatting
**Crates:** `crates/ui-egui`, `crates/format`, `crates/text` · **Spec:** [`localization-parity.md`](localization-parity.md)
- **Done:** `en` + all 8 Tier-1 scientific locales (`de`, `es`, `fr`, `ja`, `ko`, `pt-BR`, `zh-CN`, `zh-TW` — `9` locales × `167` keys = `1,503` verified translations, `0` missing in `i18n_theme`), decimal comma vs. period formatting decoupled from OS locale, `;`-CSV presets, and `PValueFormat`.
- **Missing (`8–12 h`):** Tier-2 locales (`it`, `nl`, `pl`, `ru`, `tr`, ...), RTL layout mirroring (`ar`, `he`), desktop CJK system font fallback verification, and live OS pasteboard decimal round-trips.

### G14 Headless batch CLI, `.lctemplate` pipelines and full MCP catalog
**Crates:** `apps/lumencraft-cli`, `crates/mcp`, `crates/engine`, `apps/lumencraft` · **Spec:** [`scripting-and-automation.md`](scripting-and-automation.md)
- **Done:** `lumencraft-cli` batch runner + `.lctemplate` instantiation + `Slice 0` (`7 passed`), `lumencraft-mcp` stdio server + synced `UI_COMMANDS` catalog (`25 passed`), `--control` JSONL protocol (`apps/lumencraft` `6 passed` incl. `control_loopback`, `apps/lumencraft-web` `1 passed`, `0` Clippy warnings), and `UiApp` command dispatch (`engine` `48 passed` incl. `11` `e2e_workflows`).
- **Missing (`0 h` — verified):** Headless/automation scope is complete (Note: restricted sandboxes use the verified `jsonl_control_protocol_end_to_end_over_in_memory_stream` fallback when socket binds hit `PermissionDenied`/`TimedOut`; interactive `--in-window-menus`/`--new-instance` window flags wire with `G6`).
