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
| 1 | `S01` Numerical foundations | `crates/math`, `crates/palette` | reviewing | — | — | — |
| 2 | `S05` Document model | `crates/doc` | reviewing | — | — | — |
| 3 | `S02` Statistics | `crates/stats` | reviewing | — | — | — |
| 4 | `S03` Curve fitting and equation language | `crates/fit` | reviewing | — | — | — |
| 5 | `S04` Transforms | `crates/transform` | pending | — | — | — |
| 6 | `S06` File formats and import | `crates/format` | pending | — | — | — |
| 7 | `S08` Chart compiler and scales | `crates/chart` | pending | — | — | — |
| 8 | `S09` Text, raster, SVG and PDF output | `crates/text`, `crates/render`, `crates/svg`, `crates/pdf` | pending | — | — | — |
| 9 | `S11` Tools and app entry points | `crates/tools`, `apps/lumencraft`, `apps/lumencraft-web` | pending | — | — | — |
| 10 | `S07` Engine | `crates/engine` | pending | — | — | — |
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

## Confirmed and plausible findings

Candidates for [Known defects](../gaps.md#known-defects), most severe first. Filled in as verification completes.

| ID | Severity | Title | Verdict |
|---|---|---|---|
