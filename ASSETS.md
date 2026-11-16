# Asset attribution

Every non-code asset in this repository (icons, images, presets, example datasets) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset file is missing from this table.

**Policy (mandatory):** CurveBook contains **no GraphPad or Dotmatics iconography, images, artwork, templates, sample files, equation libraries or screenshots**. Every asset is either original work by CurveBook contributors, or third-party material under an open licence on the repository allowlist, and is attributed below.

Generated-in-code art (scientific colour palettes, marker glyphs, error-bar caps, graph previews and tool cursors) is original and has no file to list.

**Font files are never committed here.** Optional bundled SIL OFL font faces are read from an external directory only when `CURVEBOOK_FONTS_DIR` is supplied at build time.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/app-icon/LICENSE.txt` | CurveBook contributors | CurveBook | MIT OR Apache-2.0 | Icon directory license note |
| `assets/app-icon/README.md` | CurveBook contributors | CurveBook | MIT OR Apache-2.0 | Icon specification and regeneration notes |
| `assets/app-icon/source/approved-mole.png` | CurveBook contributors | Original approved raster artwork for CurveBook (1254 × 1254 px) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Archived former mole artwork; retained for provenance, unused by exports |
| `assets/app-icon/source/approved-seven-pane.png` | CurveBook contributors | User-supplied approved raster artwork for CurveBook (1536 × 1024 px) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Untouched seven-pane artwork with original alpha channel |
| `assets/app-icon/source/seven-pane-win95-1254.png` | CurveBook contributors | Built-in imagegen style edit of `assets/app-icon/source/approved-seven-pane.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Additional detailed desktop-era pixel-art variant; native 1254x1254 RGBA |
| `assets/app-icon/source/seven-pane-win95.prompt.txt` | CurveBook contributors | Original imagegen prompt and export notes | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Provenance and reproducibility notes for the pixel-art variant |
| `assets/app-icon/curvebook-win95-2508.png` | CurveBook contributors | Exact 2x nearest-neighbour export of `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Large additional main-icon artwork; 2508x2508 transparent PNG |
| `assets/app-icon/slack/curvebook.png` | CurveBook contributors | Resized from `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Original seven-pane artwork exported as a transparent 128x128 static emoji |
| `assets/app-icon/slack/curvebook-subtle.gif` | CurveBook contributors | Animated size variants of `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Original seven-pane artwork with a subtle breathing loop; transparent 128x128 GIF |
| `assets/app-icon/curvebook.svg` | CurveBook contributors | Embeds `assets/app-icon/source/approved-seven-pane.png` without modification | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Master app icon; original proportions centred in a transparent 512x512 viewBox |
| `assets/app-icon/curvebook-small.svg` | CurveBook contributors | Copy of `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Self-contained SVG for scalable-icon consumers; embeds the approved raster |
| `assets/app-icon/curvebook-1024.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | 1024x1024 master PNG |
| `assets/app-icon/curvebook-macos-512.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` with Apple icon-grid margin (824/1024) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Runtime macOS Dock/window icon |
| `assets/app-icon/curvebook.icns` | CurveBook contributors | Packed from `assets/app-icon/curvebook.svg` renders via `iconutil` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | macOS bundle icon (16–1024 px including Retina) |
| `assets/app-icon/curvebook.ico` | CurveBook contributors | Packed from `assets/app-icon/curvebook.svg` renders (16–256 px) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Windows executable/installer icon |
| `assets/app-icon/hicolor/16x16/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 16x16 |
| `assets/app-icon/hicolor/22x22/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 22x22 |
| `assets/app-icon/hicolor/24x24/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 24x24 |
| `assets/app-icon/hicolor/32x32/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 32x32 |
| `assets/app-icon/hicolor/48x48/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 48x48 |
| `assets/app-icon/hicolor/64x64/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 64x64 |
| `assets/app-icon/hicolor/96x96/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 96x96 |
| `assets/app-icon/hicolor/128x128/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 128x128 |
| `assets/app-icon/hicolor/256x256/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 256x256 |
| `assets/app-icon/hicolor/512x512/apps/org.curvebook.CurveBook.png` | CurveBook contributors | Rendered from `assets/app-icon/curvebook.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 512x512 |
| `assets/app-icon/hicolor/scalable/apps/org.curvebook.CurveBook.svg` | CurveBook contributors | Copy of `assets/app-icon/curvebook-small.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor scalable |
| `docs/images/workspace-split-fit.png` | CurveBook contributors | Deterministic workspace screenshot generated by cargo xtask screenshots | MIT OR Apache-2.0 | Light theme Split Workspace (dose-response XY table, 4PL curve fit with CI bands and SD error bars, analysis result summary, navigator, inspector) |
| `docs/images/workspace-layout-dark.png` | CurveBook contributors | Deterministic workspace screenshot generated by cargo xtask screenshots | MIT OR Apache-2.0 | Dark theme multi-panel publication LayoutSheet (linked graphs A & B, embedded table, physical mm rulers, scale bar, layout inspector) |
| `assets/ui/toolbar_menus/alignment.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar alignment vector icon |
| `assets/ui/toolbar_menus/analyze.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar analyze vector icon |
| `assets/ui/toolbar_menus/axes.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar axes vector icon |
| `assets/ui/toolbar_menus/clipboard_copy.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar clipboard_copy vector icon |
| `assets/ui/toolbar_menus/clipboard_paste.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar clipboard_paste vector icon |
| `assets/ui/toolbar_menus/data_analyzed.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar data_analyzed vector icon |
| `assets/ui/toolbar_menus/export_figure.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar export_figure vector icon |
| `assets/ui/toolbar_menus/graph_type.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar graph_type vector icon |
| `assets/ui/toolbar_menus/interpret.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar interpret vector icon |
| `assets/ui/toolbar_menus/new_project.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar new_project vector icon |
| `assets/ui/toolbar_menus/open.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar open vector icon |
| `assets/ui/toolbar_menus/pairwise.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar pairwise vector icon |
| `assets/ui/toolbar_menus/palette.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar palette vector icon |
| `assets/ui/toolbar_menus/parameters.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar parameters vector icon |
| `assets/ui/toolbar_menus/print.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar print vector icon |
| `assets/ui/toolbar_menus/redo.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar redo vector icon |
| `assets/ui/toolbar_menus/rulers.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar rulers vector icon |
| `assets/ui/toolbar_menus/save.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar save vector icon |
| `assets/ui/toolbar_menus/sheet_duplicate.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar sheet_duplicate vector icon |
| `assets/ui/toolbar_menus/sheet_freeze.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar sheet_freeze vector icon |
| `assets/ui/toolbar_menus/text_style.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar text_style vector icon |
| `assets/ui/toolbar_menus/undo.svg` | CurveBook contributors | Original minimal 16x16 geometric vector SVG icon authored for CurveBook toolbar | MIT OR Apache-2.0 | Toolbar undo vector icon |
