# LumenCraft Application Icons

Original geometric artwork representing optical dispersion ("Lumen") meeting nonlinear dose-response
curve fitting and experimental data points.

## Source Files

- `lumencraft.svg`: Master 1024x1024 vector icon used for large raster sizes (`>= 48px`), macOS `.icns`, and `lumencraft-1024.png`.
- `lumencraft-small.svg`: Simplified bold-stroke variant used for `16x16`–`32x32` freedesktop icons and `hicolor/scalable/apps/org.lumencraft.LumenCraft.svg`.

## Regenerating

From the workspace root (with `resvg` or `rsvg-convert` installed, plus `iconutil` on macOS):

```sh
packaging/icons.sh
```
