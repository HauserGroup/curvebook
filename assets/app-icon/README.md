# CurveBook Application Icons

**Artwork:** seven coloured curved panes meet at a central point: cyan, green, yellow, and coral above/right; light blue, blue, and green below/left. The lower-right region is transparent.

**Source:** `source/approved-seven-pane.png` is the untouched user-approved `1536 × 1024 px` RGBA image. Preserve its original colours, proportions, and alpha channel. No opaque background is added.

**Canvas:** `viewBox="0 0 512 512"`. The full image is fitted within the square canvas, centred vertically with transparent padding; it is neither cropped nor stretched. macOS adds the existing `824/1024` canvas inset (approximately `50 px` extra margin per side at `512 px`).

**SVGs:** `curvebook.svg` embeds the approved PNG as a data URI, making the export source self-contained. `curvebook-small.svg` is an identical copy retained for existing scalable-icon consumers. These SVGs contain raster artwork, not traced vector paths, and retain the source image's resolution limit.

**Provenance & Licence:** contributed as CurveBook app artwork under `MIT OR Apache-2.0` (see `LICENSE.txt`). The former `source/approved-mole.png` remains archived with its contributor attribution; no current export uses it.

## Files

The additional detailed pixel-art variant is `curvebook-win95-2508.png` (2508 × 2508 RGBA). It uses crisp, exact 2× nearest-neighbour scaling of the native imagegen output, `source/seven-pane-win95-1254.png` (1254 × 1254 RGBA), preserving its pixel clusters and transparent background. The prompt and export notes are in `source/seven-pane-win95.prompt.txt`. The existing approved artwork and packaged platform exports remain available.

| File | What | Used by |
|---|---|---|
| `curvebook.svg` | Self-contained SVG with a transparent 512 × 512 viewBox | Source for platform raster exports |
| `curvebook-small.svg` | Identical self-contained SVG | Linux scalable icon |
| `curvebook-1024.png` | 1024 × 1024 px transparent square canvas | Documentation and store listings |
| `curvebook-macos-512.png` | 512 px canvas with additional macOS icon-grid inset | Runtime macOS Dock/window icon asset |
| `curvebook.icns` | macOS icon set, 16–1024 px, including Retina representations | `cargo xtask bundle` / `packaging/macos/package.sh` (`CFBundleIconFile`) |
| `curvebook.ico` | Windows icon with 16, 24, 32, 48, 64, 128, and 256 px PNG entries | Executable/installer icon asset (`apps/curvebook/build.rs`, `packaging/windows/`) |
| `hicolor/<size>x<size>/apps/org.curvebook.CurveBook.png` | PNGs at 16, 22, 24, 32, 48, 64, 96, 128, 256, and 512 px | Linux/FreeBSD theme icons, web favicons, installer art, runtime and in-app branding assets |
| `hicolor/scalable/apps/org.curvebook.CurveBook.svg` | Exact copy of `curvebook-small.svg` | Linux/FreeBSD scalable theme icon and web favicon |
| `source/approved-seven-pane.png` | Untouched approved image (1536 × 1024 px) | Original artwork reference |
| `source/approved-mole.png` | Archived former artwork (1254 × 1254 px) | Historical provenance only |

## Microsoft Store listing images

`store/` holds the optional Store display images, composed from `source/approved-seven-pane.png` by `packaging/store-art.sh` (ImageMagick 7):

| File | Store slot |
|---|---|
| `store/poster-720x1080.png`, `store/poster-1440x2160.png` | 9:16 poster art (opaque dark background, artwork and wordmark) |
| `store/box-1080x1080.png`, `store/box-2160x2160.png` | 1:1 box art (opaque dark background, artwork and wordmark) |
| `store/hero-1920x1080.png`, `store/hero-3840x2160.png` | 16:9 super hero art (opaque, faint grid, no title as the Store requires; artwork kept clear of the bottom-left where the Store overlays the title) |
| `store/tile-300x300.png` | 1:1 app tile icon (transparent) |
| `store/tile-150x150.png`, `store/tile-71x71.png` | 1:1 Store display logos (transparent) |

The 1x poster, box, and hero images are downsamples of the 2x masters. The wordmark is rasterised from a system font (Avenir Next on macOS by default; override with `CURVEBOOK_STORE_FONT`), so no font file is added to the repository. These images are uploaded in Partner Center; the package logos in `packaging/windows/AppxManifest.xml.in` are separate.

## Slack emoji

- `slack/curvebook.png`: transparent 128 × 128 PNG; suggested name `:curvebook:`.
- `slack/curvebook-subtle.gif`: transparent 128 × 128 GIF; suggested name `:curvebook_breathe:`. A gentle 2.16-second breathing loop varies the fitted artwork width from 120 to 124 pixels, using a shared colour palette to avoid palette flicker.

Both exports derive from `source/seven-pane-win95-1254.png` and stay below 128 KB. Transparent margins are trimmed before fitting the artwork inside the square emoji canvas. The GIF's disposal settings clear earlier artwork as it changes size, and it loops continuously. Upload each file separately through Slack's custom emoji picker.

## How it reaches each OS

- **macOS:** use `curvebook-macos-512.png` for the runtime Dock/window icon; `curvebook.icns` is bundled into `CurveBook.app` (`CFBundleIconFile`).
- **Windows:** `apps/curvebook/build.rs` and `packaging/windows/package.ps1` embed `curvebook.ico`; the 256 px hicolor PNG is used for installer artwork and the runtime icon.
- **Linux & FreeBSD:** install `hicolor/` under `share/icons/`; retain `org.curvebook.CurveBook` for the desktop entry `Icon`, AppStream metadata, and Wayland app ID / X11 class.
- **Web:** `apps/curvebook-web/index.html` copies `scalable`, `32x32`, and `256x256` from `hicolor/`.
- **In-app branding:** use the 128 px hicolor PNG for the app bar and About box.

## Regenerating

From the workspace root (with `resvg`, `rsvg-convert`, or ImageMagick (`magick`) installed, `cargo` for `xtask ico`, plus `iconutil` on macOS):

```sh
packaging/icons.sh
packaging/store-art.sh   # Microsoft Store listing images only
```
