# Audit progress — Round 1, local-files mode

Running overview of the Round 1 code audit run from one machine. It follows the protocol in [`README.md`](README.md) with these differences:

- **Local files instead of GitHub issues.** Each slice's findings and verdicts live in [`findings/<slice>.md`](findings/). Its accountable person can post them to the slice issue later.
- **One base for every slice:** `ebcc622b32fc526c545aad8439e3e580d04b478c` (`origin/main`, 2026-11-14). Every `path:line` in `findings/` refers to this commit.
- **Reviewers and verifiers are separate agent runs.** The verifier for a slice is a fresh run that forms its own view of the cited code before reading the reviewer's reasoning. Both are Claude models (reviewer and verifier use different model sizes), so the playbook's "different model family" condition is **not** met; a human or another model family should re-verify any finding before it is fixed.
- **Failing tests** written by verifiers are committed only to local branches `audit/r1/<slice>` (one per slice rather than one per finding) and are never merged.
- **No source changes** and no edits to [`docs/gaps.md`](../gaps.md). Moving confirmed findings into Known defects stays with the coordinator.

Order follows the build order in the [`ROADMAP.md`](../../ROADMAP.md) progress log: numerical foundations, document model, statistics/fitting/transforms/formats, visual stack, tools and engine, shells (UI, CLI, MCP), build guards, then the cross-cutting slices.

## Status

| Order | Slice | Paths | Status | Findings (S1/S2/S3/S4) | Verdicts (C/P/R/D) | Updated |
|---:|---|---|---|---|---|---|
| 1 | `S01` Numerical foundations | `crates/math`, `crates/palette` | done | 1/2/4/1 | 8/0/0/0 | 2026-11-14 |
| 2 | `S05` Document model | `crates/doc` | done | 3/3/1/0 | 7/0/0/0 | 2026-11-14 |
| 3 | `S02` Statistics | `crates/stats` | done | 6/5/2/0 | 13/0/0/0 | 2026-11-14 |
| 4 | `S03` Curve fitting and equation language | `crates/fit` | done | 3/6/3/0 | 12/0/0/0 | 2026-11-14 |
| 5 | `S04` Transforms | `crates/transform` | done | 4/5/4/0 | 13/0/0/0 | 2026-11-14 |
| 6 | `S06` File formats and import | `crates/format` | done | 1/4/9/0 | 12/2/0/0 | 2026-11-14 |
| 7 | `S08` Chart compiler and scales | `crates/chart` | reviewing | — | — | 2026-11-14 |
| 8 | `S09` Text, raster, SVG and PDF output | `crates/text`, `crates/render`, `crates/svg`, `crates/pdf` | reviewing | — | — | 2026-11-14 |
| 9 | `S11` Tools and app entry points | `crates/tools`, `apps/lumencraft`, `apps/lumencraft-web` | reviewing | — | — | 2026-11-14 |
| 10 | `S07` Engine | `crates/engine` | reviewing | — | — | 2026-11-14 |
| 11 | `S10` UI shell | `crates/ui-egui` | pending | — | — | — |
| 12 | `S12` CLI and MCP | `apps/lumencraft-cli`, `crates/mcp` | pending | — | — | — |
| 13 | `S13` Build guards and test kit | `xtask`, `crates/testkit` | pending | — | — | — |
| 14 | `X1` Untrusted input, end to end | entry points → engine → I/O | pending | — | — | — |
| 15 | `X2` Data correctness, end to end | cells → analyses → save/reopen | pending | — | — | — |

Status values: `pending` → `reviewing` → `verifying` → `done`. Verdicts: C = CONFIRMED, P = PLAUSIBLE, R = REFUTED, D = DUPLICATE.

## Coverage log

What each review actually read, so a later session can see what was and was not covered.

| Slice | Files read in full | Files skimmed or skipped | Tests run |
|---|---|---|---|
| `S02` | see [`findings/S02.md`](findings/S02.md) › Scope read | — | reviewer: release probe crate (12 of 13 findings reproduced); verifier: `audit/r1/S02` `crates/stats/tests/audit_r1_s02.rs`, 9 tests, all fail at base (re-run by coordinator) |
| `S01` | see [`findings/S01.md`](findings/S01.md) › Scope read | — | reviewer: probe crate + R 4.5.2 black-box oracle; verifier: `audit/r1/S01` `crates/math/tests/audit_r1_s01.rs`, `crates/palette/tests/audit_r1_s01.rs`, all 8 fail at base (R exact integrals, analytic eigenvalues) |
| `S05` | see [`findings/S05.md`](findings/S05.md) › Scope read | — | `cargo test -p lumencraft-doc` pass; verifier: `audit/r1/S05` `audit_r1_s05.rs` in `crates/doc`, `crates/format`, `crates/stats` |
| `S03` | see [`findings/S03.md`](findings/S03.md) › Scope read (`linear.rs` not read; reviewer interrupted by a rate limit after writing its file) | Bessel functions, pharma/curves/NIST library skimmed | reviewer: probe crate, 12 probes; verifier: `audit/r1/S03` `crates/fit/tests/audit_r1_s03.rs`, all 12 fail at base (R1-S03-10 `#[ignore]`, 195 s) |
| `S04` | see [`findings/S04.md`](findings/S04.md) › Scope read | — | reviewer: probe crate (11 of 13 reproduced); verifier: `audit/r1/S04` `crates/transform/tests/audit_r1_s04.rs` |
| `S06` | see [`findings/S06.md`](findings/S06.md) › Scope read | — | `cargo test -p lumencraft-format` 43/43 pass; verifier: `audit/r1/S06` `crates/format/tests/audit_r1_s06.rs`, 5 fail at base (02, 03, 04, 06, 10) |

## Coordinator notes

- **R1-S02-02 spot check (trace-only S1).** Confirmed by the coordinator at `crates/stats/src/categorical.rs:1370`–`:1378`: Fisher, Cochran–Armitage and McNemar fall back with `unwrap_or(res.chi_square_p)` under the requested test's label. `BarnardExact` at `:1371`–`:1374` does the same through `map_or(res.chi_square_p, …)` when Barnard is not computed; fix it with the same change.
- **Cross-slice overlaps to merge when recording.** R1-S04-06 (`transform/src/import.rs`, live path) and R1-S06-12 (`format/src/csv.rs`, an unused copy) both report `PlacementMode::InsertRows` overwriting rows: fix together; R1-S04-01 and R1-S05-01 share a root cause (ragged tables pass load validation).
- **Verifier depth.** The S02 and S06 verifiers each ran in about two minutes. Their failing tests were re-run and hold; their trace-only verdicts are marked `trace` below.

## Confirmed and plausible findings

Candidates for [Known defects](../gaps.md#known-defects), most severe first, generated from the verdicts in `findings/`. "Basis" is `test` when the verifier cites a failing test and `trace` when it traced the code; trace-only S1 verdicts deserve a second look before fixing.

| ID | Severity (agreed) | Title | Verdict | Basis | Slice |
|---|---|---|---|---|---|
| [R1-S01-01](findings/S01.md) | S1 | Noncentral *t* large-δ branch uses `E[U]·df/2` as the mean, so CDF/SF and Cohen's *d* CIs are wrong for \|δ\| > 447 | CONFIRMED | test | `S01` |
| [R1-S02-01](findings/S02.md) | S1 | Contingency counts near `u64` range hang the 2×2 path and panic the R×C path | CONFIRMED | test | `S02` |
| [R1-S02-02](findings/S02.md) | S1 | "Fisher's exact", "McNemar's" and "Cochran–Armitage" rows silently show the Pearson chi-square P | CONFIRMED | trace | `S02` |
| [R1-S02-03](findings/S02.md) | S1 | ROC: perfectly separated groups report P = 1 and a zero-width CI for AUC = 1 | CONFIRMED | test | `S02` |
| [R1-S02-04](findings/S02.md) | S1 | Dunn's test after Kruskal–Wallis, and "Dunn" after ordinary ANOVA, report unadjusted P as "Adjusted P" | CONFIRMED | test | `S02` |
| [R1-S02-05](findings/S02.md) | S1 | Cox PH: a singular information matrix ships SE = 1000, or SE = 1 from an identity fallback, and non-convergence is never reported | CONFIRMED | test | `S02` |
| [R1-S02-06](findings/S02.md) | S1 | One-tailed Mann–Whitney and Wilcoxon P-values ignore the direction of the effect and contradict the one-tailed *t* tests | CONFIRMED | test | `S02` |
| [R1-S03-01](findings/S03.md) | S1 | Poisson and robust fits report profile CIs exactly half the Wald width (profile computed on the wrong objective) | CONFIRMED | test | `S03` |
| [R1-S03-02](findings/S03.md) | S1 | Profile-likelihood bound falls back to the bracket midpoint when Brent fails | CONFIRMED | test | `S03` |
| [R1-S03-03](findings/S03.md) | S1 | ROUT in the independent-curves (96-curve batch) path pools the residual scale across curves | CONFIRMED | test | `S03` |
| [R1-S04-03](findings/S04.md) | S1 | Import placement grows tables without a row cap before validating | CONFIRMED | test | `S04` |
| [R1-S05-01](findings/S05.md) | S1 | Ragged tables pass load validation; `row_count()` reads only the first column and consumers index the others → panic | CONFIRMED | test | `S05` |
| [R1-S05-02](findings/S05.md) | S1 | `transpose_cell_grid` turns a ragged grid into a dense `max_cols × rows` grid with no cap, so a ~200 KB CSV can abort the app | CONFIRMED | test | `S05` |
| [R1-S05-03](findings/S05.md) | S1 | `XColumnMode::Numbers { subcolumn_mode: RawReplicates(n) }` is not bounded after load, so saving loops `n` times | CONFIRMED | test | `S05` |
| [R1-S06-01](findings/S06.md) | S1 | `.prism` import has no project-wide cell budget: a 6 KB file allocates 1.5 GiB and scales to hundreds of GiB | CONFIRMED | trace | `S06` |
| [R1-S01-02](findings/S01.md) | S2 | Noncentral F and χ² cap the Poisson mode at 500 000, so λ > 1e6 gives CDF = SF = 0 and a truncated upper CI limit | CONFIRMED | test | `S01` |
| [R1-S01-03](findings/S01.md) | S2 | `sym_eigen` declares convergence against an absolute threshold floored at 1.0, so small-scale matrices stop after one Jacobi sweep | CONFIRMED | test | `S01` |
| [R1-S02-07](findings/S02.md) | S2 | Zero residual variance gives F = 0, P = 1 in RM ANOVA, and P = 1 for every post-hoc pair after an F test with P < 0.0001 | CONFIRMED | test | `S02` |
| [R1-S02-08](findings/S02.md) | S2 | ANOVA on mean/SD/N data reports a made-up Brown–Forsythe "F = 0.000, P = 1.0000 — Pass" | CONFIRMED | trace | `S02` |
| [R1-S02-09](findings/S02.md) | S2 | Cox regression treats any non-zero status as an event; Kaplan–Meier rejects the same column | CONFIRMED | trace | `S02` |
| [R1-S02-10](findings/S02.md) | S2 | Outlier "Row Index" and per-column scalar indices count only non-blank cells, not table rows or data sets | CONFIRMED | trace | `S02` |
| [R1-S02-11](findings/S02.md) | S2 | ROUT flags nothing when 68% or more of the values are tied | CONFIRMED | test | `S02` |
| [R1-S03-04](findings/S03.md) | S2 | Implicit-equation solver uses absolute tolerances and a fixed search range | CONFIRMED | test | `S03` |
| [R1-S03-05](findings/S03.md) | S2 | `hyp1f1` and `hyp2f1` return truncated partial sums as values | CONFIRMED | test | `S03` |
| [R1-S03-06](findings/S03.md) | S2 | Robust (Cauchy) fits report SEs, CIs, bands and AICc/F comparisons that the guide says are not applicable | CONFIRMED | test | `S03` |
| [R1-S03-07](findings/S03.md) | S2 | Batch (independent-curves) path accepts curves with fewer points than parameters and reports SE 0, R² 1, Sy.x 0 | CONFIRMED | test | `S03` |
| [R1-S03-08](findings/S03.md) | S2 | `Y0 = expr` in a non-ODE equation is silently discarded and `Y0` becomes a fitted parameter | CONFIRMED | test | `S03` |
| [R1-S03-09](findings/S03.md) | S2 | DSL compile cost grows quadratically with source size, with no total cap | CONFIRMED | test | `S03` |
| [R1-S04-01](findings/S04.md) | S2 | Ragged tables panic the formula evaluator and Transpose | CONFIRMED | test | `S04` |
| [R1-S04-02](findings/S04.md) | S2 | Long left-associative formula chains overflow the stack in the recursive evaluator | CONFIRMED | test | `S04` |
| [R1-S04-04](findings/S04.md) | S2 | Bare built-in constants such as `e` silently resolve to default-labelled columns | CONFIRMED | test | `S04` |
| [R1-S04-05](findings/S04.md) | S2 | Import placement never adds columns: AppendColumns and wide pastes silently drop data | CONFIRMED | test | `S04` |
| [R1-S04-06](findings/S04.md) | S2 | `InsertRows` placement overwrites existing rows instead of inserting | CONFIRMED | test | `S04` |
| [R1-S04-07](findings/S04.md) | S2 | Prune with averaging averages SD and N subcolumns and wipes text variables | CONFIRMED | test | `S04` |
| [R1-S04-08](findings/S04.md) | S2 | Simulate allocates up to 512 M cells from user parameters with no total budget | CONFIRMED | test | `S04` |
| [R1-S04-09](findings/S04.md) | S2 | XY Transpose and Select→XY projection silently keep only subcolumn 0 | CONFIRMED | test | `S04` |
| [R1-S05-04](findings/S05.md) | S2 | `clone_structure` with `override_replicates` turns `Mean/SD/N` cells into raw replicates | CONFIRMED | test | `S05` |
| [R1-S05-05](findings/S05.md) | S2 | A `Column` table's subcolumn mode is not checked against its kind after load; extra replicates are ignored without a message | CONFIRMED | test | `S05` |
| [R1-S05-06](findings/S05.md) | S2 | `ordered_levels_with` is quadratic in the number of distinct levels | CONFIRMED | test | `S05` |
| [R1-S06-02](findings/S06.md) | S2 | Checkpoint writes the snapshot before truncating the journal and records no covered sequence, so recovery replays commands already in the snapshot | CONFIRMED | test | `S06` |
| [R1-S06-03](findings/S06.md) | S2 | One malformed interior journal line aborts all recovery, and the desktop app then refuses to start | CONFIRMED | test | `S06` |
| [R1-S06-04](findings/S06.md) | S2 | PZFX import silently drops subcolumns beyond the resolved subcolumn mode | CONFIRMED | test | `S06` |
| [R1-S06-05](findings/S06.md) | S2 | `.lumencraft` open parses `document.json` into a `serde_json::Value` tree first: a 61 KB archive drives 1.1 GiB of heap | CONFIRMED | trace | `S06` |
| [R1-S01-04](findings/S01.md) | S3 | `cholesky` detects singularity only by `pivot <= 0`, so exactly collinear Gram matrices factor and invert to ~1e14 | CONFIRMED | test | `S01` |
| [R1-S01-05](findings/S01.md) | S3 | Inverse incomplete gamma/beta solvers return the bracket midpoint as `Ok` after the iteration budget | CONFIRMED | test | `S01` |
| [R1-S01-06](findings/S01.md) | S3 | `reg_beta_inv_pair` derives the second component as `1 − x`, losing all precision when the quantile is near 1 | CONFIRMED | test | `S01` |
| [R1-S01-07](findings/S01.md) | S3 | `ColorRamp::Custom` copies and sorts every stop on every sample, and the stop count is unbounded | CONFIRMED | test | `S01` |
| [R1-S02-12](findings/S02.md) | S3 | Mann–Whitney does not match its documented exactness: ties always use the normal approximation, and large-sample "Hodges–Lehmann" is a difference of medians | CONFIRMED | trace | `S02` |
| [R1-S02-13](findings/S02.md) | S3 | RM one-way ANOVA, Friedman and Cox drop incomplete rows without saying so, and Cox ignores `reference_levels` | CONFIRMED | trace | `S02` |
| [R1-S03-10](findings/S03.md) | S3 | ODE models re-integrate from `t0` per point with a per-evaluation step ceiling and no overall budget | CONFIRMED | test | `S03` |
| [R1-S03-11](findings/S03.md) | S3 | References to non-existent predictors (`X2`, `X3`, …) silently evaluate to 0 | CONFIRMED | test | `S03` |
| [R1-S03-12](findings/S03.md) | S3 | Interpolated unknowns extrapolate up to 50 % of the X span without a flag; documented out-of-range flags do not exist | CONFIRMED | test | `S03` |
| [R1-S04-10](findings/S04.md) | S3 | Curve calculus pairs each data set's compacted series with data set A's X values | CONFIRMED | test | `S04` |
| [R1-S04-11](findings/S04.md) | S3 | Remove Baseline blanks cells for missing or zero baselines without a warning | CONFIRMED | test | `S04` |
| [R1-S04-12](findings/S04.md) | S3 | The simulation seed in the summary tab loses precision above 2^53 | CONFIRMED | test | `S04` |
| [R1-S04-13](findings/S04.md) | S3 | A non-ASCII, non-alphanumeric character produces a "token limit" parse error | CONFIRMED | test | `S04` |
| [R1-S05-07](findings/S05.md) | S3 | `SummarizedPoint::reconstruct` returns `Ok` for impossible moments | CONFIRMED | test | `S05` |
| [R1-S06-06](findings/S06.md) | S3 | Save validates finiteness only for cells, constants and results; a non-finite `f64` elsewhere writes a file that cannot be reopened | CONFIRMED | test | `S06` |
| [R1-S06-08](findings/S06.md) | S3 | An unparsable `lumencraft.lock` is a permanent acquire error rather than a stale lease | CONFIRMED | trace | `S06` |
| [R1-S06-09](findings/S06.md) | S3 | `MemoryJournalBackend` writes web-storage JSON it later refuses to read back | CONFIRMED | trace | `S06` |
| [R1-S06-10](findings/S06.md) | S3 | One overlong PZFX row title rejects the whole import, while other titles are truncated with a warning | CONFIRMED | test | `S06` |
| [R1-S06-11](findings/S06.md) | S3 | XML node cap counts elements only; text nodes and attributes give about 10× memory amplification | CONFIRMED | trace | `S06` |
| [R1-S06-12](findings/S06.md) | S3 | `PlacementMode::InsertRows` overwrites the rows at the anchor instead of inserting | CONFIRMED | trace | `S06` |
| [R1-S06-13](findings/S06.md) | S3 | `apply_csv_import_link` header promotion maps header index to data-set index, ignoring subcolumns | CONFIRMED | trace | `S06` |
| [R1-S06-07](findings/S06.md) | S3 | Stale-lock eviction is unlink-then-create: two concurrent recoverers can both acquire the lease | PLAUSIBLE | trace | `S06` |
| [R1-S06-14](findings/S06.md) | S3 | Journal replay is unbounded in size and record count | PLAUSIBLE | trace | `S06` |
| [R1-S01-08](findings/S01.md) | S4 | `gamma(-inf)` returns `Ok(NaN)` despite the documented error contract | CONFIRMED | test | `S01` |
