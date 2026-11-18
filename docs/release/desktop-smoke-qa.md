# CurveBook Beta Desktop Smoke & Manual QA Walkthrough (`R10.24`)

This document is the canonical cross-OS release smoke checklist for native `CurveBook` desktop installers and binaries on **macOS** (`.dmg` / `.app`), **Windows** (`.msi` / `.exe`), and **Linux** (`.AppImage` / `.deb` / `.tar.gz`). Run every section cleanly against candidate builds prior to Beta tag sign-off.

Automated interactive regression backing for every step below is tracked in [GUI Workflows](../gui-workflows.md) and [UI Feature Parity](../ui-parity.md).

---

## 1. Packaging, Launch & Visual Fidelity

| Platform | Package Artifact | Smoke Verification |
|---|---|---|
| **macOS** (`aarch64` / `x86_64`) | `CurveBook-*.dmg` | Mount `.dmg`, drag `CurveBook.app` to Applications, launch via Finder; confirm clean app icon, menu bar (`CurveBook`, `File`, `Edit`, `Analyze`, `Graph`, `View`, `Help`), Retina (`2.0x`/`3.0x`) crisp text/hairlines, and `--headless-once` CLI exit `0`. |
| **Windows** (`x86_64`) | `CurveBook-*.msi` / `-setup.exe` | Run installer cleanly without admin warnings on per-user install, launch from Start Menu, resize/maximize across primary and HiDPI monitors (`1.25x`/`2.0x`), confirm DWM native title bar and clean quit. |
| **Linux** (`x86_64` / `aarch64`) | `CurveBook-*.AppImage` / `.deb` | Run `chmod +x CurveBook-*.AppImage && ./CurveBook-*.AppImage` (and `apt install` smoke for `.deb`), confirm `--in-window-menus` fallback chrome works without missing shared libraries on X11/Wayland. |

### Theme, RTL & Language Checks
1. Toggle `Light`, `Dark`, and `HighContrast` themes; confirm zero white-on-white or black-on-black controls, crisp contrast on `MultipleVariables` role badges (`NUM`, `CAT`, `LBL`, `FX=`), and clean focus rings.
2. Open **View › Language**, scroll the full 16-entry list: verify `"English (en)"` has **no** `[Draft]` suffix while all 15 non-English locales carry `[Draft]`.
3. Select **Arabic (`ar`)** or **Hebrew (`he`)**: confirm top toolbar, Navigator/Inspector drawers, grid columns, and modal headers mirror into RTL cleanly; switch back to `en`.

---

## 2. File Dialogs, Drag-and-Drop & Archive Persistence

1. **Welcome & Empty Project**: Verify the Welcome & New Data Table Launcher displays all 8 table archetypes, `Tutorial Sample Data`, and `.cbtemplate` starter templates. Click **Open Existing File…** and verify the native OS open picker filters for `.curvebook`, `.cbtemplate`, `.prism`, `.pzfx`, `.csv`, and `.tsv`.
2. **`File › Open…` & Format Interop**:
   - Open a `.curvebook` project archive and verify sheets and graphs load intact.
   - Open a `.pzfx` XML project and verify multi-table import (`XY`, `Column`, `Grouped`, `Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`, `Nested`), exclusions, and `InfoSheet` constants.
   - Open a `.prism` ZIP container archive and verify table/info import without mutating any previously saved `.curvebook` path (`file_path` detaches safely).
   - Open a `.csv` delimited table and verify columns populate.
3. **Drag-and-Drop**: Drag a `.pzfx` file and a `.prism` file directly onto the desktop window canvas; confirm immediate clean import and rendering.
4. **`File › Save` & `File › Save As…`**:
   - Trigger `File › Save` on an unsaved project; verify native save picker opens, writes valid `.curvebook` (or `.curvebook.d` unpacked directory), clears dirty indicator, and subsequent `File › Save` writes directly without re-prompting.
   - Trigger `File › Save As…`; verify the native save picker always prompts for a new path and updates the active window title/archive target.
5. **Vector & Raster Export**: On an initialized `GraphSheet` or `LayoutSheet`, trigger `File › Export › SVG…`, `File › Export › PDF…`, and `File › Export › PNG…`; inspect the written files (`<svg`, `%PDF-1.7`, `\x89PNG`).

---

## 3. Data Grid Editing, Paste & Quick-Stats Footer

1. **Inline Cell Editing (`X` & `Y`)**:
   - Double-click an editable `X` cell on an `XY` sheet, type / compose IME characters (e.g. Unicode `"濃度"` or `"7.5"`), verify live inline buffer updates before commit, press `Enter`, and confirm the committed number/text persists.
   - Repeat on a `Y` data cell; verify pressing `Escape` restores the pre-edit cell value without recording an undo entry.
2. **Row Headers & Sheet Titles**:
   - Click a row title cell (`grid.row_title.*`) -> verify `Escape` cancels cleanly with zero mutation, then type a new row title and press `Enter` -> verify commit.
   - Double-click a sheet entry in the **Project Navigator** (`nav.sheet.*`) -> verify live inline buffer edit box appears, `Escape` cancels cleanly, and typing a new title + `Enter` commits via `sheet.rename`.
3. **`Cmd/Ctrl+V` Paste & Quick-Stats Footer**:
   - Copy multi-cell TSV values (`"10\t20\n30\t40"`) to the OS clipboard, focus a DataSheet grid cell, press `Cmd+V` (macOS) or `Ctrl+V` (Windows/Linux), and confirm tabular paste commits into the grid.
   - Drag-select a numeric column range (`[10, 20, 30, 40]`) and verify the bottom status-bar footer displays live **`n`**, **`excl`**, **`mean`**, **`SD`**, **`SEM`**, **`median`**, and **`sum`**.

---

## 4. `MultipleVariables` Picker, Portfolio Gallery & `F6` Pane Cycling

1. **Portfolio Gallery**: Click the toolbar **Open Browser / Gallery** button (or toggle Gallery), click folder filter tabs (`All`, `Data`, `Info`, `Results`, `Graphs`, `Layouts`), wheel-scroll the card grid, and single-click a card to switch to that sheet and dismiss the gallery.
2. **`F6` / `Shift+F6` Pane Cycling**:
   - Press `F6` repeatedly and verify the visible focus border steps cleanly across only **visible** panes (`Navigator`, `FamilyStrip`, `MainCanvasOrGrid`, `Inspector`, plus `FloatingNotes` when present).
   - Hide Inspector (`Cmd/Ctrl+Shift+I`) and Family (`Cmd/Ctrl+Shift+F`), press `F6`, and confirm hidden panes are skipped cleanly; verify `Shift+F6` reverses order.
3. **`MultipleVariables` Data-Tab Variable Picker**:
   - On a `MultipleVariables` sheet, open **Analyze…** and `Cmd/Ctrl+T` (**Edit Analysis Parameters**).
   - Verify the **Role Filter** chips (`All`, `Continuous`, `Categorical`, `Label`, `Calculated`) filter the variable list cleanly, colour-coded role badges (`NUM`, `CAT`, `LBL`, `FX=`) match variable roles, and toggling variable checkboxes updates analysis parameters and recomputes results.

---

## 5. Shortcuts, Vector Clipboard Copy & Crash Recovery

1. **Shortcuts**: Verify `Cmd/Ctrl+Z` (undo), `Cmd/Ctrl+Shift+Z` (redo), and `Cmd/Ctrl+Shift+{N,I,F}` pane toggles respond immediately from keyboard input.
2. **Multi-Flavor Copy (`Cmd/Ctrl+C`)**: On an active `GraphSheet` and an active `LayoutSheet`, press `Cmd/Ctrl+C` and paste into a vector editor or office document consumer to verify `%PDF-1.7`, `SVG`, `HTML`, `PNG`, and `EMF` flavors.
3. **Crash Recovery & Close Guard**:
   - Create >64 cell edits on an unsaved project (evicting the oldest entry of the 64-entry undo buffer) via inline GUI edits, force-terminate (`kill -9` / Task Manager End Task) without saving, relaunch, and verify the recovery banner restores the 65th+ GUI edit intact without duplicate tables.
   - Verify clicking the close button with unsaved edits shows the **Save / Don't Save / Cancel** modal; closing cleanly (`Don't Save` or after `Save`) releases the recovery directory lock and clears the pending crash-recovery journal.

---

## 6. Automated Verification Commands (Before Tagging)

Run inside the workspace with clean environment before Beta sign-off:

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
