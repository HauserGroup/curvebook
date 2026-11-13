#!/usr/bin/env bash
# Regenerate rasterized/platform icons in assets/app-icon/ from lumencraft.svg and lumencraft-small.svg.
# Requires: resvg (or rsvg-convert), cargo (for xtask ico), and iconutil on macOS (for .icns).
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

ICON_DIR="$ROOT/assets/app-icon"
APP_ID="org.lumencraft.LumenCraft"

render_svg() {
  local svg="$1" size="$2" out="$3"
  mkdir -p "$(dirname "$out")"
  if command -v resvg >/dev/null; then
    resvg -w "$size" -h "$size" "$svg" "$out"
  elif command -v rsvg-convert >/dev/null; then
    rsvg-convert -w "$size" -h "$size" "$svg" -o "$out"
  else
    echo "error: need resvg or rsvg-convert to render $svg" >&2
    exit 1
  fi
}

render_svg "$ICON_DIR/lumencraft.svg" 1024 "$ICON_DIR/lumencraft-1024.png"
render_svg "$ICON_DIR/lumencraft.svg" 512 "$ICON_DIR/lumencraft-macos-512.png"

for sz in 16 24 32 48 64 128 256 512; do
  src="$ICON_DIR/lumencraft.svg"
  if [[ "$sz" -le 32 ]]; then src="$ICON_DIR/lumencraft-small.svg"; fi
  render_svg "$src" "$sz" "$ICON_DIR/hicolor/${sz}x${sz}/apps/${APP_ID}.png"
done
mkdir -p "$ICON_DIR/hicolor/scalable/apps"
cp "$ICON_DIR/lumencraft-small.svg" "$ICON_DIR/hicolor/scalable/apps/${APP_ID}.svg"

(cd "$ROOT" && cargo xtask ico "$ICON_DIR/lumencraft.ico" \
  "$ICON_DIR/hicolor/16x16/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/24x24/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/32x32/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/48x48/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/256x256/apps/${APP_ID}.png")

if command -v iconutil >/dev/null; then
  ICONSET="$(mktemp -d)/lumencraft.iconset"
  mkdir -p "$ICONSET"
  for sz in 16 32 128 256 512; do
    render_svg "$ICON_DIR/lumencraft.svg" "$sz" "$ICONSET/icon_${sz}x${sz}.png"
    render_svg "$ICON_DIR/lumencraft.svg" "$((sz * 2))" "$ICONSET/icon_${sz}x${sz}@2x.png"
  done
  iconutil -c icns -o "$ICON_DIR/lumencraft.icns" "$ICONSET"
  rm -rf "$(dirname "$ICONSET")"
fi
echo "icons: updated $ICON_DIR"
