## Roadmap item or issue

<!-- Planned work: R<m>.<n> from docs/ROADMAP.md; the PR title is plain English ending in (R<m>.<n>).
     Reported bug outside planned work: fill in the issue number below, put the same "Fixes #<N>" trailer in the commit message, and leave the issue number out of the title. -->

Fixes #

## What changed

<!-- Concise summary of the implementation, docs, or fix and why this approach was chosen. -->

## Verified

<!-- Exact commands, statistical/oracle dataset checks, and interactive UI/CLI/MCP checks run after your final edit. -->

## Not verified

<!-- State explicitly what was skipped, requires another OS, or waits on downstream tasks. Write "None" if everything applicable ran cleanly. -->

## Checklist

- [ ] Issue fixes only: a regression test named `issue_<N>_*` failed before the fix and passes after it (a `D1` oracle with a cited reference for numerical results), and no existing oracle tolerance or reference value was loosened without a reason under **Not verified**.
- [ ] `cargo xtask ci quick` passes locally. The Workspace CI gate runs the full default suite on GitHub.
- [ ] No panics in shipped code: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!` outside tests, and no unchecked indexing on files, parameters, or messages. Errors return typed `Result` values ([AGENTS.md](../AGENTS.md)).
- [ ] Clean-room & neutral wording: nothing copied from proprietary commercial statistical/graphing applications or `GPL`/`AGPL`/`LGPL` codebases, and no competitor or placeholder suite brand names; lowercase `.pzfx files` and `.prism files` naming is specifically allowed and preferred where appropriate ([AGENTS.md](../AGENTS.md)).
- [ ] Every new asset has a row in `ASSETS.md` (and `ATTRIBUTION.md` when third-party), with zero `.ttf`/`.otf`/`.woff`/`.woff2` font files added.
- [ ] Any generated artefacts modified (`ui_catalog.rs`, `commands.md`, `schemas/*.schema.json`) were regenerated cleanly and `docs/ROADMAP.md` / `docs/STATUS.md` reflect the change accurately.
