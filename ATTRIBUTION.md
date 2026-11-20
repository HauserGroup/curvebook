# Attribution

The complete asset manifest is [ASSETS.md](ASSETS.md). Every non-code asset in this repository uses a license from the repository's allowlist (`MIT`, `Apache-2.0`, `ISC`, `BSD-2-Clause`, `BSD-3-Clause`, `OFL-1.1`, `CC0-1.0`, `CC-BY-4.0`), with full license notices preserved next to the asset or summarized in [`NOTICE`](NOTICE).

| Asset | Author | Source | SPDX license | Notes |
|---|---|---|---|---|
| `assets/app-icon/source/approved-mole.png` | CurveBook contributors | Original approved raster artwork created for CurveBook (1254 × 1254 px) | MIT OR Apache-2.0 | Archived former engraved mole artwork; contributor credit retained |
| `assets/app-icon/curvebook.svg` | CurveBook contributors | Embeds `assets/app-icon/source/approved-seven-pane.png` for CurveBook | MIT OR Apache-2.0 | Canonical 512x512 viewBox master app icon source |
| `assets/app-icon/source/approved-seven-pane.png` | CurveBook contributors | User-supplied approved seven-pane artwork for CurveBook (1536 × 1024 px) | MIT OR Apache-2.0 | Untouched source with original transparency |
| `assets/app-icon/source/seven-pane-win95-1254.png` | CurveBook contributors | Built-in imagegen style edit of the approved seven-pane artwork | MIT OR Apache-2.0 | Additional detailed pixel-art version; prompt retained alongside the source |
| `assets/app-icon/curvebook-win95-2508.png` | CurveBook contributors | Exact 2x pixel scaling of the generated 1254x1254 source | MIT OR Apache-2.0 | Transparent large-format pixel-art export |
| `assets/app-icon/slack/curvebook.png` | CurveBook contributors | Resized from `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 | Transparent static emoji derived from original CurveBook artwork |
| `assets/app-icon/slack/curvebook-subtle.gif` | CurveBook contributors | Animated from `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 | Transparent animated emoji derived from original CurveBook artwork |

## Attributed Source Code

Per `AGENTS.md` (*Attributed VectorCraft GUI source reuse*), CurveBook adapts permissively licensed (`MIT OR Apache-2.0`) pure-`egui` widget helpers and top-bar width-fitting logic from upstream repository `https://github.com/storytold/vectorcraft` at commit `644f5bc8e377730d093ac5af8967dcd055051a16`:

- `crates/ui-egui/src/widgets/toggles.rs` -> [`crates/ui-egui/src/egui_shell/widgets.rs`](crates/ui-egui/src/egui_shell/widgets.rs) (MIT OR Apache-2.0)
- `crates/ui-egui/src/widgets/buttons.rs` -> [`crates/ui-egui/src/egui_shell/widgets.rs`](crates/ui-egui/src/egui_shell/widgets.rs) (MIT OR Apache-2.0)
- `crates/ui-egui/src/widgets/labels.rs` -> [`crates/ui-egui/src/egui_shell/widgets.rs`](crates/ui-egui/src/egui_shell/widgets.rs) (MIT OR Apache-2.0)
- `crates/ui-egui/src/widgets/panel_chrome.rs` -> [`crates/ui-egui/src/egui_shell/widgets.rs`](crates/ui-egui/src/egui_shell/widgets.rs) (MIT OR Apache-2.0)
- `crates/ui-egui/src/widgets/wheel_stream.rs` -> [`crates/ui-egui/src/egui_shell/widgets.rs`](crates/ui-egui/src/egui_shell/widgets.rs) (MIT OR Apache-2.0)
- `crates/ui-egui/src/chrome.rs` (`fit_right_group`) -> [`crates/ui-egui/src/egui_shell/top_bar_fit.rs`](crates/ui-egui/src/egui_shell/top_bar_fit.rs) (MIT OR Apache-2.0)
