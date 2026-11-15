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
| 1 | `S01` Numerical foundations | `crates/math`, `crates/palette` | done | 1/2/4/1 | 8/0/0/0 | 2026-11-15 |
| 2 | `S05` Document model | `crates/doc` | done | 3/3/1/0 | 7/0/0/0 | 2026-11-15 |
| 3 | `S02` Statistics | `crates/stats` | done | 6/5/2/0 | 13/0/0/0 | 2026-11-15 |
| 4 | `S03` Curve fitting and equation language | `crates/fit` | done | 3/6/3/0 | 12/0/0/0 | 2026-11-15 |
| 5 | `S04` Transforms | `crates/transform` | done | 4/5/4/0 | 13/0/0/0 | 2026-11-15 |
| 6 | `S06` File formats and import | `crates/format` | done | 1/4/9/0 | 12/2/0/0 | 2026-11-15 |
| 7 | `S08` Chart compiler and scales | `crates/chart` | done | 2/6/3/0 | 11/0/0/0 | 2026-11-15 |
| 8 | `S09` Text, raster, SVG and PDF output | `crates/text`, `crates/render`, `crates/svg`, `crates/pdf` | done | 1/3/6/0 | 10/0/0/0 | 2026-11-15 |
| 9 | `S11` Tools and app entry points | `crates/tools`, `apps/lumencraft`, `apps/lumencraft-web` | done | 1/5/6/1 | 12/1/0/0 | 2026-11-15 |
| 10 | `S07` Engine | `crates/engine` | reviewing | 5/4/1/0 | — | 2026-11-15 |
| 11 | `S10` UI shell | `crates/ui-egui` | verifying | 2/3/3/1 | — | 2026-11-15 |
| 12 | `S12` CLI and MCP | `apps/lumencraft-cli`, `crates/mcp` | verifying | 4/2/3/1 | — | 2026-11-15 |
| 13 | `S13` Build guards and test kit | `xtask`, `crates/testkit` | verifying | 0/2/8/0 | — | 2026-11-15 |
| 14 | `X1` Untrusted input, end to end | entry points → engine → I/O | reviewing | — | — | 2026-11-15 |
| 15 | `X2` Data correctness, end to end | cells → analyses → save/reopen | reviewing | 0/2/0/0 | — | 2026-11-15 |

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
| `S08` | see [`findings/S08.md`](findings/S08.md) › Scope read | — | `cargo test -p lumencraft-chart` 35/35 pass; reviewer: probe crate (debug + release); verifier: `audit/r1/S08` (`c15abdc`) `crates/chart/tests/audit_r1_s08.rs`; 07 and 11 traced only |
| `S09` | see [`findings/S09.md`](findings/S09.md) › Scope read | — | slice tests pass (55); reviewer: probe binary; verifier: `audit/r1/S09` (`5a0e8ee`), tests in `text`, `pdf`, `svg`, `render`; 08 and 10 traced only |
| `S11` | see [`findings/S11.md`](findings/S11.md) › Scope read | — | slice tests pass; reviewer: `--control` probe + scratch engine crate; verifier: `audit/r1/S11` `apps/lumencraft/tests/audit_r1_s11.rs` (02, 05, 13 fail at base; rest traced) |
| `S06` | see [`findings/S06.md`](findings/S06.md) › Scope read | — | `cargo test -p lumencraft-format` 43/43 pass; verifier: `audit/r1/S06` `crates/format/tests/audit_r1_s06.rs`, 5 fail at base (02, 03, 04, 06, 10) |

## Coordinator notes

- **R1-S02-02 spot check (trace-only S1).** Confirmed by the coordinator at `crates/stats/src/categorical.rs:1370`–`:1378`: Fisher, Cochran–Armitage and McNemar fall back with `unwrap_or(res.chi_square_p)` under the requested test's label. `BarnardExact` at `:1371`–`:1374` does the same through `map_or(res.chi_square_p, …)` when Barnard is not computed; fix it with the same change.
- **Cross-slice overlaps to merge when recording.** R1-S04-06 (`transform/src/import.rs`, live path) and R1-S06-12 (`format/src/csv.rs`, an unused copy) both report `PlacementMode::InsertRows` overwriting rows: fix together; R1-S04-01 and R1-S05-01 share a root cause (ragged tables pass load validation).
- **Verifier depth.** The S02 and S06 verifiers each ran in about two minutes. Their failing tests were re-run and hold; their trace-only verdicts are marked `trace` below.

- **R1-S11-01 spot check (trace-only S1).** Confirmed by the coordinator: the only non-test callers of the journal write path are the engine wrappers in `crates/engine/src/lib.rs:162`–`:237`; their one caller in `apps/lumencraft/src/main.rs:574` sits inside `mod tests` (`:448`). The desktop app never records a command.
- **Rate-limit interruption (2026-11-14).** Reviewers for S07, S08, S09 and S11 were interrupted with no output and rerun from scratch; S03's reviewer was interrupted after writing a complete findings file, which was kept.
- **Unrelated edits in the main checkout.** While this audit ran, another session left uncommitted feature edits (`QOL-5`, `QOL-8`, CLI, engine) in the main checkout's working tree on this branch. Audit commits stage only `docs/audit/`; reviewers read code only from the base worktree.

- **`main` moved past the base during the audit.** After `a993740`, ten commits landed on `main` (`7b5bafa`..`47704e8`, 214 files, including "stats/fit remediation" and "Koopman/pharma parity"). The audit stays at `ebcc622`. Verdict lines reading "Still present on main: yes (main == base)" were true when written; re-check each finding against current `main` before fixing it, since some may already be fixed or moved. Audit bookkeeping moved to its own worktree on branch `audit/r1-local-2` so it no longer shares the main checkout.

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
| [R1-S08-01](findings/S08.md) | S1 | Log-axis tick generator overflows and allocates without bound when a data value is near `f64::MAX` or the smallest subnormal | CONFIRMED | test | `S08` |
| [R1-S08-02](findings/S08.md) | S1 | Y-axis title placement panics in `f64::clamp` on canvases narrower than about 40 pt | CONFIRMED | test | `S08` |
| [R1-S09-01](findings/S09.md) | S1 | Raster dashing has no length budget: one long dashed annotation line allocates without bound and aborts the app | CONFIRMED | test | `S09` |
| [R1-S11-01](findings/S11.md) | S1 | The desktop app opens a crash-recovery journal but never records a command, so a crash loses all work since launch | CONFIRMED | test | `S11` |
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
| [R1-S08-03](findings/S08.md) | S2 | Default replicate beeswarm is quadratic: a 16,000-value column takes 8 s per render, a 100,000-value column several minutes | CONFIRMED | test | `S08` |
| [R1-S08-04](findings/S08.md) | S2 | XY/Column auto ranges on probit, logit and reciprocal axes exclude valid data, and the omitted points raise no warning | CONFIRMED | test | `S08` |
| [R1-S08-05](findings/S08.md) | S2 | An explicit reversed axis range (`min > max`) is silently replaced, hiding data | CONFIRMED | test | `S08` |
| [R1-S08-06](findings/S08.md) | S2 | Pre-summarized Mean/SD/N rows: a missing N draws ±SD for any requested metric, and a missing SD removes the mean even when no error bars are requested | CONFIRMED | test | `S08` |
| [R1-S08-07](findings/S08.md) | S2 | A significance bracket whose group is not drawn falls back to the slot at the same position and labels the wrong pair | CONFIRMED | test | `S08` |
| [R1-S08-08](findings/S08.md) | S2 | User major-tick intervals: ticks silently stop after 65 intervals, and tiny intervals overflow `i64` arithmetic | CONFIRMED | test | `S08` |
| [R1-S09-02](findings/S09.md) | S2 | `parse_markup` is quadratic on unmatched `*`: a large text-box annotation hangs every render | CONFIRMED | test | `S09` |
| [R1-S09-03](findings/S09.md) | S2 | Default (selectable-text) PDF export replaces every non-WinAnsi, non-Symbol character with `?`, even with an embedded TrueType font | CONFIRMED | test | `S09` |
| [R1-S09-04](findings/S09.md) | S2 | SVG export writes XML-forbidden control characters from labels, producing a file no XML parser accepts, while `validate_svg_structure` passes it | CONFIRMED | test | `S09` |
| [R1-S11-02](findings/S11.md) | S2 | Any crash or startup error leaves `lumencraft.lock` behind, and the app then refuses to start for 5 minutes | CONFIRMED | test | `S11` |
| [R1-S11-03](findings/S11.md) | S2 | The lease is never renewed: a second instance steals it after 5 minutes, and whichever instance quits last overwrites the other's recovery snapshot | CONFIRMED | trace | `S11` |
| [R1-S11-04](findings/S11.md) | S2 | Several files on the command line: earlier projects are silently dropped, and a CSV merged into a project is reported as unmodified | CONFIRMED | trace | `S11` |
| [R1-S11-05](findings/S11.md) | S2 | An unreadable or rejected `shortcuts.json` / `preferences.json` stops the app from starting, including a valid swapped-shortcut file | CONFIRMED | test | `S11` |
| [R1-S11-06](findings/S11.md) | S2 | `LUMENCRAFT_NO_PREFS` and `LUMENCRAFT_CONFIG_DIR` are documented but never read, so "isolated" automation runs share the user's real recovery directory | CONFIRMED | trace | `S11` |
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
| [R1-S08-09](findings/S08.md) | S3 | The guarded axis engine in `scales.rs` that the tests and parity checklist verify is not the one `compile_graph` runs | CONFIRMED | trace | `S08` |
| [R1-S08-10](findings/S08.md) | S3 | `scales.rs` reciprocal error-bar clamp uses +1e-12 as the floor on a negative axis, producing coordinates near 1e14 pt | CONFIRMED | test | `S08` |
| [R1-S08-11](findings/S08.md) | S3 | Dendrogram node ids from a result tab are negated without overflow checks | CONFIRMED | test | `S08` |
| [R1-S09-05](findings/S09.md) | S3 | PDF writer measures text with its own approximate Helvetica/Symbol width table, so centred and right-aligned PDF text is misplaced | CONFIRMED | test | `S09` |
| [R1-S09-06](findings/S09.md) | S3 | Multi-line text is broken into lines only by the raster/outline paths; selectable PDF and editable SVG put all lines on one baseline | CONFIRMED | test | `S09` |
| [R1-S09-07](findings/S09.md) | S3 | With `BUNDLED_FONTS` empty, raster and outlined text draw every accented Latin letter as an empty box | CONFIRMED | test | `S09` |
| [R1-S09-08](findings/S09.md) | S3 | BiDi reordering is applied only by the PDF writer; PNG/TIFF and outlined SVG draw RTL text in logical order | CONFIRMED | trace | `S09` |
| [R1-S09-09](findings/S09.md) | S3 | cmap format 4 collection iterates every code of every segment even when nothing is kept: a 262 KB font takes 3.2 s to validate | CONFIRMED | test | `S09` |
| [R1-S11-07](findings/S11.md) | S3 | Shortcut and preference changes are never saved by the shipped app | CONFIRMED | test | `S11` |
| [R1-S11-09](findings/S11.md) | S3 | With `HOME` unset, config and recovery fall back to fixed, shared paths under `/tmp` | CONFIRMED | trace | `S11` |
| [R1-S11-10](findings/S11.md) | S3 | `GridToolState::emit_exclude_selection_intent` sends reading-order bounds that the engine reads as a 3-D box (latent over- and under-exclusion) | CONFIRMED | trace | `S11` |
| [R1-S11-11](findings/S11.md) | S3 | Navigator drag commits by the index captured at drag start and discards the dragged sheet ID (latent stale-index reorder) | CONFIRMED | trace | `S11` |
| [R1-S11-12](findings/S11.md) | S3 | Drag-to-place comparison brackets guess the compared groups from `x / 48 pt` (latent wrong annotation) | CONFIRMED | trace | `S11` |
| [R1-S06-07](findings/S06.md) | S3 | Stale-lock eviction is unlink-then-create: two concurrent recoverers can both acquire the lease | PLAUSIBLE | trace | `S06` |
| [R1-S06-14](findings/S06.md) | S3 | Journal replay is unbounded in size and record count | PLAUSIBLE | trace | `S06` |
| [R1-S11-08](findings/S11.md) | S3 | OS clipboard adapter waits for EOF on `xclip`/`wl-copy` stdout, which the backgrounded clipboard owner keeps open (latent hang) | PLAUSIBLE | trace | `S11` |
| [R1-S01-08](findings/S01.md) | S4 | `gamma(-inf)` returns `Ok(NaN)` despite the documented error contract | CONFIRMED | test | `S01` |
| [R1-S09-10](findings/S09.md) | S4 | Compound-glyph decoding has a depth cap but no total budget: 64 components per level × 8 levels is exponential (latent, bundled fonts only) | CONFIRMED | trace | `S09` |
| [R1-S11-13](findings/S11.md) | S4 | Startup environment and flags: a malformed `LUMENCRAFT_CONTROL_PORT` aborts a launch that never asked for control; `--new-instance` and paths beginning with `-` are not handled | CONFIRMED | test | `S11` |
