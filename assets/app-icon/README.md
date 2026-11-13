# LumenCraft Application Icons

**Creature:** a dark engraved mole holding a triangular refractive prism. A white beam enters the prism and emerges as a rainbow spectrum.

**Style:** black-and-ivory engraving with directional fur strokes, hatching, and stippling. Tight head-and-shoulders portrait on a full-bleed purple field, with the body running off the tile.

**Palette:** multicolour, preserving the purple background and rainbow spectrum. Representative solid colours in the master export:

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#1a1716` | Dark fur, contours, and engraving |
| Paper | `#f0e8d6` | Highlights, claws, snout, and prism |
| Purple | `#512073` | Full-bleed field |
| Red | `#f31d21` | Spectrum |
| Orange | `#fb8802` | Spectrum |
| Yellow | `#f8d603` | Spectrum |
| Green | `#23b638` | Spectrum |
| Cyan | `#03b0f5` | Spectrum |
| Blue | `#233bd0` | Spectrum |
| Violet | `#9b37f5` | Spectrum |

Additional tonal colours occur in the tracing and antialiasing.

**Tile:** `viewBox="0 0 512 512"`, clipped to a rounded square with `rx="112"`. Windows and Linux use the tile edge to edge. macOS uses a centred tile measuring `824/1024` of the canvas width (`412 px` tile with `50 px` transparent margins on the `512 px` canvas).

**Provenance & Licence:** `source/approved-mole.png` is the untouched `1254 × 1254 px` approved raster source. `lumencraft.svg` (traced at `2048 px`) is the canonical vector for platform exports, and `lumencraft-small.svg` (traced at `1024 px`) is the lighter scalable SVG. Dual-licensed under `MIT OR Apache-2.0` (see `LICENSE.txt`).

## Files

| File | What | Used by |
|---|---|---|
| `lumencraft.svg` | Master path-based vector, traced at 2048 px | Source for platform raster exports |
| `lumencraft-small.svg` | Lighter path-based vector, traced at 1024 px | Linux scalable icon |
| `lumencraft-1024.png` | Full tile, 1024 × 1024 px | Documentation and store listings |
| `lumencraft-macos-512.png` | 512 px canvas; 412 px tile with 50 px transparent margins | Runtime macOS Dock/window icon |
| `lumencraft.icns` | macOS icon set, 16–1024 px, including Retina representations | `cargo xtask bundle` / `packaging/macos/package.sh` (`CFBundleIconFile`) |
| `lumencraft.ico` | Windows icon with 16, 24, 32, 48, 64, 128, and 256 px PNG entries | Embedded executable/installer icon (`apps/lumencraft/build.rs`, `packaging/windows/`) |
| `hicolor/<size>x<size>/apps/org.lumencraft.LumenCraft.png` | PNGs at 16, 22, 24, 32, 48, 64, 96, 128, 256, and 512 px | Linux/FreeBSD theme icons, web favicons, installer art, runtime and in-app branding |
| `hicolor/scalable/apps/org.lumencraft.LumenCraft.svg` | Exact copy of `lumencraft-small.svg` | Linux/FreeBSD scalable theme icon and web favicon |
| `source/approved-mole.png` | Untouched approved final image (1254 × 1254 px) | Original artwork reference |

## How it reaches each OS

- **macOS:** use `lumencraft-macos-512.png` for the runtime Dock/window icon; `lumencraft.icns` is bundled into `LumenCraft.app` (`CFBundleIconFile`).
- **Windows:** `apps/lumencraft/build.rs` and `packaging/windows/package.ps1` embed `lumencraft.ico`; the 256 px hicolor PNG is used for installer artwork and the runtime icon.
- **Linux & FreeBSD:** install `hicolor/` under `share/icons/`; retain `org.lumencraft.LumenCraft` for the desktop entry `Icon`, AppStream metadata, and Wayland app ID / X11 class.
- **Web:** `apps/lumencraft-web/index.html` copies `scalable`, `32x32`, and `256x256` from `hicolor/`.
- **In-app branding:** use the 128 px hicolor PNG for the app bar and About box.

## Regenerating

From the workspace root (with `resvg` or `rsvg-convert` installed, `cargo` for `xtask ico`, plus `iconutil` on macOS):

```sh
packaging/icons.sh
```
