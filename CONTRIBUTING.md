# Contributing to LumenCraft

Thank you for helping. This page tells you how to pick up work, what must never happen in this repository, and what a pull request needs before it can merge.

The rules of record are in [`AGENTS.md`](AGENTS.md). They apply equally to people and to AI agents. Where this page and `AGENTS.md` disagree, `AGENTS.md` wins.

## Ways to help

You do not need to write Rust to be useful here.

- **Code:** take a step from the [execution order](docs/roadmap.md#execution-order).
- **Reference values:** produce reference numbers for statistical methods that have no public benchmark yet. This needs R or SciPy and domain knowledge. See [Reference values](#reference-values).
- **Review:** read open pull requests. Review, not writing code, limits how fast this project moves.
- **Testing:** run the app or the CLI on data you own and report what breaks or looks wrong.

## Before you start

1. Read [`AGENTS.md`](AGENTS.md), then [`ROADMAP.md`](ROADMAP.md) and the [execution order](docs/roadmap.md#execution-order).
2. Pick work from the **current step** of the execution order, inside the scope of the next gate. Work the next gate does not need waits, however interesting it is.
3. Before touching a crate, read its implementation guide in [`docs/`](docs/). The list is in [`docs/development.md`](docs/development.md#subsystem-implementation-guides).

## Rules that cannot be broken

These protect the project's legal standing and its users' data. A pull request that breaks one will be closed.

- **Clean room.** Never launch, script, screenshot, or measure GraphPad Prism or any Dotmatics application for this project, and never compare our output against it. Never read or copy anything from an installation of it, and never commit files it produced. Behaviour comes from public documentation, textbooks, and peer-reviewed methods, cited by URL or reference.
- **No copyleft code.** Never copy or port GPL, AGPL, or LGPL source (R's `stats`, the GNU Scientific Library, JASP, PSPP, and similar). Dependencies and assets must be on the [licence allowlist](AGENTS.md#licence-allowlist).
- **No panics in shipped code.** Fallible work returns `Result` or `Option`. `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, and `unimplemented!` are banned outside tests, and so is unchecked indexing on data from files, parameters, or messages. See [why and how](docs/development.md#robustness-lumencraft-never-crashes).
- **Every asset is attributed.** A new asset file needs a row in [`ASSETS.md`](ASSETS.md), and in [`ATTRIBUTION.md`](ATTRIBUTION.md) when it is third-party. Font files are never committed.
- **No secrets or personal details** in the repository, logs, issues, or requests to outside services. Do not attach unpublished or confidential research data to an issue.

## Claiming work

1. Find or open the issue for the step or gap you want.
2. Search open pull requests and recent commits on `main` for the same work.
3. Comment on the issue to claim it **before** you start.
4. If you stop, say so on the issue so someone else can take it.

## Setting up

You need Rust 1.95 or later. Optional tools and the full command list are in [`docs/development.md`](docs/development.md).

```sh
cargo xtask ci        # every quality gate; run before each commit
cargo xtask test      # unit, integration and doc tests only
cargo xtask corpus    # verify the NIST StRD reference corpus (--fetch to download it)
```

`cargo xtask ci` runs formatting, clippy with warnings denied, tests, doctests, and the asset, brand, clean-room, cursor, UI-command, layer, and WebAssembly checks. The WebAssembly check is skipped locally when the `wasm32-unknown-unknown` target is not installed, and is enforced in CI.

If several people or agents build in the same checkout, give each its own `CARGO_TARGET_DIR`.

## Making a change

- **Keep it narrow.** One gap or one step per pull request. Fix what the issue describes, where the problem is.
- **Test it.** A bug fix adds a test that fails without the fix. A statistical test or curve-fitting model is checked against an analytical solution or a named reference, with an explicit tolerance.
- **Name the oracle.** A numerical pull request says which reference each test uses. The accepted references are listed under [verification oracles](docs/roadmap.md#verification-oracles).
- **Everything is a command.** User-visible behaviour is a `CommandSpec` in `crates/engine` with tests. The UI stays thin and acts through commands.
- **Respect the layers.** `cargo xtask layers` rejects upward dependencies. See [`docs/architecture.md`](docs/architecture.md).
- **Update the docs in the same pull request** when a task lands: the roadmap files, the gap list, and the relevant parity checklist. `AGENTS.md` lists them.
- **Read a guard before changing it.** A check that looks removable says what it prevents.

## Pull requests

- `main` changes only through a pull request. Do not push to it.
- Fill in the [pull request template](.github/pull_request_template.md). Say what you tested and what you could not run.
- Review your own diff first: identifiers exist as used, nothing unrelated changed, and the description is true.
- Pull requests are squash-merged, and the pull request title becomes the commit title. Start it with the task id, for example `G3: paired t-test with effect sizes` or `docs: clarify corpus fetch`.
- Keep changes to CI workflows in their own pull request unless the change requires them.
- Answer review feedback, or say that you cannot. Do not stack new work in a crate while an earlier pull request there is unreviewed.

More detail is in the [landing rules](docs/roadmap.md#landing-rules).

## Using AI agents

Agents are welcome, and the same rules apply to them.

- **A person is accountable.** Whoever runs an agent answers for what it opens and for its review comments.
- **Report only what ran.** If the agent could not build or test locally, the pull request says so and relies on CI.
- **No agent attribution.** A pull request does not say that an agent wrote it.

## Reference values

Many statistical methods have no public benchmark dataset. For those, reference values produced by open-source statistical software such as R or SciPy may be committed as test fixtures. The limits are in the clean-room section of [`AGENTS.md`](AGENTS.md); in short:

- Run the software only as a black box. Never read or port its source.
- Use synthetic data, NIST StRD data, or data from a cited publication.
- Record the software and package, the version, the exact call, and the date next to the values.
- The software never becomes a build, test, or CI dependency.

## Licensing of contributions

LumenCraft is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at the user's option. Unless you state otherwise, any contribution you intentionally submit for inclusion is licensed the same way, without additional terms or conditions.

## Credits

The About window lists contributors from generated commit statistics. To set how your name appears, or to opt out, see [`docs/contributors.md`](docs/contributors.md). Only edit your own entry.

## Security

Do not report a vulnerability in a public issue. See [`SECURITY.md`](SECURITY.md).
