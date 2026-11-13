## What and why

<!-- What does this change, and why? Link the issue if there is one. -->

## How it was tested

<!-- Statistical / numerical reference checks, tests added, and how you checked behaviour (CLI, MCP, UI). -->

## Checklist

- [ ] `cargo xtask ci` passes (fmt, clippy, tests, doctests, assets, brands, cleanroom, cursors, ui-commands, layers, wasm).
- [ ] No panics in shipped code: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or
      `unimplemented!` outside tests, and no unchecked indexing on data from files, parameters or
      messages. Errors are returned as `Result` and shown to the user
      ([why and how](../docs/development.md#robustness-lumencraft-never-crashes)).
- [ ] Clean-room: nothing copied from GraphPad / Dotmatics products or GPL/AGPL/LGPL code ([AGENTS.md](../AGENTS.md)).
- [ ] Every new asset has a row in `ASSETS.md` (and `ATTRIBUTION.md` when third-party).
