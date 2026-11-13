# Asset attribution

Every non-code asset in this repository (icons, images, presets, example datasets) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset file is missing from this table.

**Policy (mandatory):** LumenCraft contains **no GraphPad or Dotmatics iconography, images, artwork, templates, sample files, equation libraries or screenshots**. Every asset is either original work by LumenCraft contributors, or third-party material under an open licence on the repository allowlist, and is attributed below.

Generated-in-code art (scientific colour palettes, marker glyphs, error-bar caps, graph previews and tool cursors) is original and has no file to list.

**Font files are never committed here.** Optional bundled SIL OFL font faces are read from an external directory only when `LUMENCRAFT_FONTS_DIR` is supplied at build time.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/app-icon/LICENSE.txt` | LumenCraft contributors | LumenCraft | MIT OR Apache-2.0 | Icon directory license note |
| `assets/app-icon/README.md` | LumenCraft contributors | LumenCraft | MIT OR Apache-2.0 | Icon regeneration notes |
| `assets/app-icon/lumencraft.svg` | LumenCraft contributors | Original geometric SVG drawn in code for LumenCraft | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (dispersive prism + dose-response spectrum curve) |
| `assets/app-icon/lumencraft-small.svg` | LumenCraft contributors | Original simplified geometric SVG drawn in code for LumenCraft | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Scalable small-size app icon |
| `assets/app-icon/lumencraft-1024.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | 1024x1024 master PNG |
| `assets/app-icon/lumencraft-macos-512.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` with Apple icon-grid margin | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Runtime macOS Dock/window icon |
| `assets/app-icon/lumencraft.icns` | LumenCraft contributors | Packed from `assets/app-icon/lumencraft.svg` renders via `iconutil` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | macOS bundle icon |
| `assets/app-icon/lumencraft.ico` | LumenCraft contributors | Packed from `assets/app-icon/lumencraft.svg` renders via `cargo xtask ico` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Windows executable/installer icon |
| `assets/app-icon/hicolor/16x16/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 16x16 |
| `assets/app-icon/hicolor/24x24/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 24x24 |
| `assets/app-icon/hicolor/32x32/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 32x32 |
| `assets/app-icon/hicolor/48x48/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 48x48 |
| `assets/app-icon/hicolor/64x64/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 64x64 |
| `assets/app-icon/hicolor/128x128/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 128x128 |
| `assets/app-icon/hicolor/256x256/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 256x256 |
| `assets/app-icon/hicolor/512x512/apps/org.lumencraft.LumenCraft.png` | LumenCraft contributors | Rendered from `assets/app-icon/lumencraft.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor 512x512 |
| `assets/app-icon/hicolor/scalable/apps/org.lumencraft.LumenCraft.svg` | LumenCraft contributors | Copy of `assets/app-icon/lumencraft-small.svg` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Freedesktop hicolor scalable |
