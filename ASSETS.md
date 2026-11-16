# Asset attribution

Every non-code asset in this repository (icons, images, presets, example datasets) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset file is missing from this table.

**Policy (mandatory):** LumenCraft contains **no GraphPad or Dotmatics iconography, images, artwork, templates, sample files, equation libraries or screenshots**. Every asset is either original work by LumenCraft contributors, or third-party material under an open licence on the repository allowlist, and is attributed below.

Generated-in-code art (scientific colour palettes, marker glyphs, error-bar caps, graph previews and tool cursors) is original and has no file to list.

**Font files are never committed here.** Optional bundled SIL OFL font faces are read from an external directory only when `LUMENCRAFT_FONTS_DIR` is supplied at build time.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/app-icon/LICENSE.txt` | LumenCraft contributors | LumenCraft | MIT OR Apache-2.0 | Icon directory license note |
| `assets/app-icon/README.md` | LumenCraft contributors | LumenCraft | MIT OR Apache-2.0 | Icon specification and regeneration notes |
| `assets/app-icon/source/approved-mole.png` | LumenCraft contributors | Original approved raster artwork for LumenCraft (1254 × 1254 px) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Archived former mole artwork; retained for provenance, unused by exports |
| `assets/app-icon/source/approved-seven-pane.png` | LumenCraft contributors | User-supplied approved raster artwork for LumenCraft (1536 × 1024 px) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Untouched seven-pane artwork with original alpha channel |
| `assets/app-icon/source/seven-pane-win95-1254.png` | LumenCraft contributors | Built-in imagegen style edit of `assets/app-icon/source/approved-seven-pane.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Additional detailed desktop-era pixel-art variant; native 1254x1254 RGBA |
| `assets/app-icon/source/seven-pane-win95.prompt.txt` | LumenCraft contributors | Original imagegen prompt and export notes | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Provenance and reproducibility notes for the pixel-art variant |
| `assets/app-icon/lumencraft-win95-2508.png` | LumenCraft contributors | Exact 2x nearest-neighbour export of `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Large additional main-icon artwork; 2508x2508 transparent PNG |
| `assets/app-icon/slack/lumencraft.png` | LumenCraft contributors | Resized from `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Original seven-pane artwork exported as a transparent 128x128 static emoji |
| `assets/app-icon/slack/lumencraft-subtle.gif` | LumenCraft contributors | Animated size variants of `assets/app-icon/source/seven-pane-win95-1254.png` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Original seven-pane artwork with a subtle breathing loop; transparent 128x128 GIF |
| `assets/app-icon/lumencraft.svg` | LumenCraft contributors | Embeds `assets/app-icon/source/approved-seven-pane.png` without modification | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Master app icon; original proportions centred in a transparent 512x512 viewBox |
| `assets/app-icon/lumencraft-small.svg` | LumenCraft contributors | Copy of `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Self-contained SVG for scalable-icon consumers; embeds the approved raster |
| `assets/app-icon/lumencraft-1024.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | 1024x1024 master PNG |
| `assets/app-icon/lumencraft-macos-512.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` with Apple icon-grid margin (824/1024) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Runtime macOS Dock/window icon |
| `assets/app-icon/lumencraft.icns` | LumenCraft contributors | Packed from `assets/app-icon/lumencraft.svg` renders via `iconutil` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | macOS bundle icon (16–1024 px including Retina) |
| `assets/app-icon/lumencraft.ico` | LumenCraft contributors | Packed from `assets/app-icon/lumencraft.svg` renders (16–256 px) | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Windows executable/installer icon |
| `assets/app-icon/hicolor/16x16/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 16x16 |
| `assets/app-icon/hicolor/22x22/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 22x22 |
| `assets/app-icon/hicolor/24x24/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 24x24 |
| `assets/app-icon/hicolor/32x32/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 32x32 |
| `assets/app-icon/hicolor/48x48/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 48x48 |
| `assets/app-icon/hicolor/64x64/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 64x64 |
| `assets/app-icon/hicolor/96x96/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 96x96 |
| `assets/app-icon/hicolor/128x128/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 128x128 |
| `assets/app-icon/hicolor/256x256/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 256x256 |
| `assets/app-icon/hicolor/512x512/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 512x512 |
| `assets/app-icon/hicolor/scalable/apps/org.lumencraft.LumenCraft.svg` | LumenCraft contributors | Copy of `assets/app-icon/lumencraft-small.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor scalable |
| `docs/images/workspace-split-fit.png` | LumenCraft contributors | Deterministic workspace screenshot generated by cargo xtask screenshots | MIT OR Apache-2.0 | Light theme Split Workspace (dose-response XY table, 4PL curve fit with CI bands and SD error bars, analysis result summary, navigator, inspector) |
| `docs/images/workspace-layout-dark.png` | LumenCraft contributors | Deterministic workspace screenshot generated by cargo xtask screenshots | MIT OR Apache-2.0 | Dark theme multi-panel publication LayoutSheet (linked graphs A & B, embedded table, physical mm rulers, scale bar, layout inspector) |
