# Contributor Credits

CurveBook builds its in-app **About -> Contributors** credit list from two tracked files under [`contributors/`](../contributors/):

- [`contributors/people.toml`](../contributors/people.toml): Optional opt-in display overrides keyed by GitHub login (`real_name`, `display_name`, or `unlisted = true`). Never add another person's real name without their explicit consent via pull request.
- [`contributors/contributors.json`](../contributors/contributors.json): Machine-generated contributor roster refreshed via `cargo xtask contributors` (and run automatically by `.github/workflows/release.yml` prior to release builds).
