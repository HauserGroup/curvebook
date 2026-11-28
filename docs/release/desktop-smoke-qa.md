# CurveBook Desktop Smoke & Manual QA Walkthrough (`R10.24`)

This document is the canonical cross-OS smoke walkthrough for `CurveBook` Beta candidate installers on **macOS** (universal `.dmg`), **Windows 11** (`x64` / `arm64` unsigned `.msi`), and **Linux** (`x86_64` / `aarch64` `.AppImage`), aligned with the [`R0.28`](../history/roadmap/r0.md#r028-rein-in-the-number-of-release-install-files) single-format-per-OS Beta download set and [`R10.24`](../ROADMAP.md#r1024-windows--linux-desktop-gui-smoke-qa-10-gate).

> **Gate (updated 2026-11-20):** this recorded walkthrough is a **1.0** requirement. **Beta** relies on automated completion of all `65` workflows through the drawn desktop GUI ([`R10.25`](../history/roadmap/r10.md#r1025-automated-gui-completion-of-all-65-workflows-beta-gate)) and does not wait on this walkthrough. Run it on release candidates as they become available.

Automated interactive regression backing is tracked in [GUI Workflows](../gui-workflows.md) and [UI Feature Parity](../evidence/user-interface.md). Known Beta issues (including Windows SmartScreen guidance) are tracked in [Troubleshooting & Known Issues](https://curvebook.org/manual/latest/troubleshooting.html#known-issues).

> [!IMPORTANT]
> **Desktop Recording & Acceptance Rule (`R10.24`):** Execute this walkthrough using computer-use (mouse and screen) or manual human operation on real or virtual desktop sessions (GitHub-hosted CI runners have no interactive display session, and semantic `--control` / `--event-script` / MCP harness runs never count per [Pitfall #6](../pitfalls.md#6-treating-semantic-agent-harness-tests-as-proof-of-human-gui-parity)). Record the exact **OS version**, **installer/build artifact**, **scaling/display-server configuration**, **steps**, and **results** in [`STATUS.md`](../STATUS.md), and file any regressions against `R10.23` or new `R10` items.

---

## 1. Beta Candidate Matrix, Sessions & Scaling (`R0.28` / `R10.24`)

| Platform | Beta Release Asset (`R0.28`) | Required Desktop Environment, Portal & Scaling Checks |
|---|---|---|
| **macOS** (`universal`: `aarch64` + `x86_64`) | `curvebook-<version>-macos-universal.dmg` (signed & notarized under `R8.10`, bundles `CurveBook.app` + `curvebook-cli`) | Mount `.dmg`, drag `CurveBook.app` to Applications, launch via Finder; verify menu bar (`File`, `Edit`, `View`, `Insert`, `Data`, `Analyze`, `Graph`, `Help`, plus the macOS `CurveBook` app menu), Retina scaling (`2.0x` / `3.0x`), bundled `curvebook-cli --help`, and `CurveBook.app --headless-once` exit `0`. |
| **Windows 11** (`x64`, `arm64`) | `curvebook-<version>-windows-x64.msi`, `curvebook-<version>-windows-arm64.msi` (**unsigned** until `R8.11`) | Install `.msi` on `x64` (and `arm64` where hardware is available); observe and **record the Windows SmartScreen prompt** for Beta release notes; test at **100%** and **200%** display scaling, window resize/maximize, and crisp HiDPI text/hairlines. |
| **Linux** (`x86_64`, `aarch64`) | `curvebook-<version>-linux-x86_64.AppImage`, `curvebook-<version>-linux-aarch64.AppImage` *(only Linux format in the `R0.28` Beta set)* | `chmod +x` and run the `.AppImage` on both a **Wayland** session and an **X11** session using the **XDG file-chooser portal** at **1x** and **2x** scaling; confirm `--in-window-menus` chrome works without missing shared libraries. |
| **FreeBSD** (`x86_64`) | `curvebook-<version>-freebsd-x86_64.tar.gz` *(Tier 2 CLI-only)* | Tier 2 CLI-only — desktop GUI is not supported on FreeBSD. |

### Theme, RTL & Language Checks
1. Toggle `Light`, `Dark`, and `HighContrast` themes; confirm zero white-on-white or black-on-black controls, legible contrast on `MultipleVariables` role badges (`NUM`, `CAT`, `LBL`, `FX=`), and visible focus rings.
2. Open **View › Language** and scroll the Beta-active locales: confirm `"English (en)"` has **no** `[Draft]` suffix while translated non-English Beta locales display `[Draft]` (RTL locales `ar` and `he` stay hidden from the desktop picker in Beta and are deferred to `1.0`).
3. *(1.0 RTL gate, when enabled)* Select **Arabic (`ar`)** or **Hebrew (`he`)**: confirm toolbar, Navigator/Inspector drawers, grid columns, and modal headers mirror into RTL cleanly; switch back to `en`.

---

## 2. End-to-End Beta Desktop Walkthrough

Execute the first-run core desktop probe (`R10.13` / `R10.37`), then complete every stage below sequentially on each target OS session:

### First-Run Core Desktop Probe (`R10.13` / `R10.37`)
1. **Import a dose-response workbook:** Launch CurveBook and import an `.xlsx` dose-response workbook via the Welcome **Import** view (or **File › Import**).
2. **Fit and inspect live update after a cell edit:** Run a `4PL` nonlinear dose-response curve fit on the imported table, edit a dose-response measurement cell directly in the rendered grid (`Enter` to commit), and confirm the linked result scalars (`EC50`, `R^2`) and fitted curve redraw immediately.
3. **Export from both a graph and a layout:** Open unified **Export Figure** from both the linked `GraphSheet` (toolbar **Export Figure** or **File › Export › Export Figure**) and a `LayoutSheet` containing that graph, choose a format (`SVG` / `PDF` / `PNG`) inside the dialog, commit via the native save dialog, and verify both output files.

### Full Walkthrough Stages

1. **Install & Launch from Candidate**: Install/run the OS Beta candidate above and confirm clean startup to the **Welcome to CurveBook** launcher.
2. **Start a Project from Welcome Hub (`R10.37`)**:
   - Confirm the left rail puts **Open**, **Import**, **New Data Table**, and **Sample Data** first, followed by grouped **Reuse an Analysis** (**New from Template** and **Clone Pipeline** tabs).
   - Under **New Data Table**, inspect the 7 core archetypes (`XY`, `Column`, `Grouped`, `Contingency`, `Survival`, `Multiple Variables`, `Nested`), expand **Additional table types** to confirm **Parts of Whole** is present, and inspect **Sample Data** and **Reuse an Analysis** (curated `.cbtemplate` starters + `Open .cbtemplate File` and **Clone Pipeline**).
   - Create a typed table (e.g., `XY` with concentration/response columns, plus repeat on `MultipleVariables` confirming **Analyze** and the formula bar stay directly on the MV toolbar while **Data › Randomize Groups** and **Data › Sample Rows** run cleanly from the **Data** menu), and verify saving as a template via **File › Save Template** remains available from the open project.
3. **Enter, Edit & Paste Data**:
   - Double-click an `X` cell (`GridXCell`) and `Y` data cell (`GridCell`), type/compose IME input (e.g., `"7.5"` or `"濃度"`), verify live inline editing buffer updates, commit with `Enter`, and confirm `Escape` cancels an edit cleanly without mutating the cell or recording an undo entry.
   - Click a row title (`grid.row_title.*`) -> cancel with `Escape`, then rename and commit with `Enter`.
   - Double-click a sheet title in the Project Navigator (`nav.sheet.*`) -> verify live inline edit box, `Escape` cancel without mutation, and `Enter` commit via `sheet.rename`.
   - Copy tabular values (`"10\t20\n30\t40"`) to the OS clipboard, focus a cell, and paste via `Cmd+V` (macOS) / `Ctrl+V` (Windows/Linux).
   - Drag-select numeric cells (`[10, 20, 30, 40]`) and verify the bottom status-bar footer (`status.summary`) displays live **`n`**, **`excl`**, **`mean`**, **`SD`**, **`SEM`**, **`median`**, and **`sum`**.
4. **Run an Analysis & a Nonlinear Fit**:
   - Click **Analyze…**, run a group/statistical analysis (e.g., One-way ANOVA or two-group comparison), press `Cmd/Ctrl+T` to adjust parameters in **Edit Analysis Parameters**, and verify live recomputation.
   - On a `MultipleVariables` sheet, open **Analyze…** and `Cmd/Ctrl+T`: verify the role filter chips (`All`, `Continuous`, `Categorical`, `Label`, `Calculated`) filter variables cleanly and colour-coded role badges (`NUM`, `CAT`, `LBL`, `FX=`) match variable roles.
   - Run a **nonlinear fit** (e.g., `4PL` sigmoidal dose-response or `One-Phase Exponential Decay` from the built-in equation catalog), verify fit summary scalars (`R^2`, `EC50`/rate constant, `95% CI`) and assumption diagnostic banner (`QOL-7`), and confirm parameter constraints / comparison via `Cmd/Ctrl+T`.
5. **Make a Graph, `View › Gallery` & `F6` Pane Cycling**:
   - Open or create a `GraphSheet` showing the data and fitted curve; test Inspector formatting (`Format Graph` / `Format Axes`) and `Cmd/Ctrl+Z` undo / `Cmd/Ctrl+Shift+Z` redo.
   - Confirm no Gallery launch control appears in the footer or Navigator folder headers; open the **Portfolio Gallery** via its single entry point **View › Gallery**, filter by folder (`All`, `Data`, `Info`, `Results`, `Graphs`, `Layouts`), wheel-scroll the card grid, click a graph thumbnail to open it directly (confirming Welcome does not reappear), and multi-select graphs (`Shift` / `Cmd/Ctrl+Click`) to verify bulk **Graph › Apply Style From**.
   - Cycle focus with `F6` and `Shift+F6`: confirm focus moves strictly across visible panes, and hiding Inspector (`Cmd/Ctrl+Shift+I`) or Family (`Cmd/Ctrl+Shift+F`) skips hidden panes cleanly.
6. **Native File Dialogs (`Open`, `Save`, `Save As`), Drag-and-Drop & Reopen**:
   - Trigger `File › Save` and save the project as `.curvebook` via the native OS file picker (confirm XDG file-chooser portal on Linux Wayland & X11); confirm dirty indicator clears and subsequent `File › Save` writes in place.
   - Trigger `File › Save As…` and verify the native picker always prompts for a new path.
   - **Close and reopen the saved `.curvebook` project** (via `File › Open…` or relaunching and opening); verify every data sheet, analysis, nonlinear fit curve, and graph renders identically.
   - Test opening and dragging-and-dropping `.pzfx`, `.prism`, `.csv`, and `.curvebook` files onto the window canvas (verifying foreign imports detach the native `.curvebook` archive path safely).
7. **Export Figures (`SVG`, `PDF`, `PNG`, `TIFF`, `EMF`) through Unified `Export Figure`**:
   - On an initialized `GraphSheet` and on a `LayoutSheet`, open the unified **Export Figure** dialog via **File › Export › Export Figure** and via the graph/layout toolbar **Export Figure** button (confirming duplicate per-format `SVG`/`PDF`/`PNG`/`TIFF` entries are absent from **File › Export**, while CSV/JSON/`.pzfx`/Methods Report exports stay available).
   - Select `SVG`, `PDF`, and `PNG` inside **Export Figure**, commit through native save dialogs, and inspect the exported files (`<svg`, `%PDF-1.7`, PNG signature & dimensions).
8. **Copy Graph & Paste into Another Application**:
   - Focus an active `GraphSheet` (and `LayoutSheet`), press `Cmd/Ctrl+C`, and paste into an external vector editor or office document consumer to verify multi-flavor vector/raster clipboard delivery (`%PDF-1.7`, `SVG`, `HTML`, `PNG`, `EMF`).
9. **Open Print Preview (`File › Print…`)**:
   - Trigger `File › Print…`, verify page/margin/orientation controls render cleanly, launch OS/host print preview, and verify clean preview generation without blocking errors.
10. **Crash Recovery, Clean Close & Uninstall**:
    - Perform >64 inline GUI cell edits (evicting the oldest entry of the 64-entry undo buffer) on an unsaved project, force-terminate without saving, relaunch, and verify the crash-recovery banner restores the 65th+ GUI edit intact without duplicate sheets.
    - Close cleanly with unsaved changes to verify the **Save / Don't Save / Cancel** guard releases the recovery lock and clears the pending journal.
    - **Uninstall cleanly** from the OS:
      - **macOS**: Move `CurveBook.app` from `/Applications` to Trash and confirm clean removal.
      - **Windows 11**: Uninstall `CurveBook` via *Settings › Apps › Installed apps* (or `msiexec /x`) and confirm clean removal with no leftover Start Menu or broken shortcuts.
      - **Linux**: Remove the executable `.AppImage` (and desktop integration entry if created) cleanly.

---

## 3. Automated Pre-Tag Verification Commands

Run inside the workspace with clean environment before Beta tag sign-off:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown -p curvebook-ui-egui -- -D warnings
cargo test --workspace
cargo xtask ui-commands --check
cargo xtask i18n-report
cargo xtask status
cargo xtask brands
cargo xtask cleanroom
```
