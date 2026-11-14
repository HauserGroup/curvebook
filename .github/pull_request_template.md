## Roadmap item

<!-- R<m>.<n> from docs/ROADMAP.md (link issue/claim if applicable). PR title must be plain English ending in (R<m>.<n>). -->

## What changed

<!-- Concise summary of the implementation, docs, or fix and why this approach was chosen. -->

## Verified

<!-- Exact commands, statistical/oracle dataset checks, and interactive UI/CLI/MCP checks run after your final edit. -->

## Not verified

<!-- State explicitly what was skipped, requires another OS, or waits on downstream tasks. Write "None" if everything applicable ran cleanly. -->

## Checklist

- [ ] `cargo xtask ci` passes (fmt, clippy, tests, doctests, layers, assets, brands, cleanroom, cursors, ui-commands, wasm, and active `R0` checks).
- [ ] No panics in shipped code: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!` outside tests, and no unchecked indexing on files, parameters, or messages. Errors return typed `Result` values ([AGENTS.md](../AGENTS.md)).
- [ ] Clean-room & neutral wording: nothing copied from proprietary commercial statistical/graphing applications or `GPL`/`AGPL`/`LGPL` codebases, and no competitor or placeholder suite brand names ([AGENTS.md](../AGENTS.md)).
- [ ] Every new asset has a row in `ASSETS.md` (and `ATTRIBUTION.md` when third-party), with zero `.ttf`/`.otf`/`.woff`/`.woff2` font files added.
- [ ] Any generated artefacts modified (`ui_catalog.rs`, `commands.md`, `schemas/*.schema.json`) were regenerated cleanly and `docs/ROADMAP.md` / `docs/STATUS.md` reflect the change accurately.
