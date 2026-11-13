#!/usr/bin/env bash
# Regenerate rasterized/platform icons in assets/app-icon/ from lumencraft.svg and lumencraft-small.svg.
# Requires: resvg (or rsvg-convert), cargo (for xtask ico), and iconutil on macOS (for .icns).
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

ICON_DIR="$ROOT/assets/app-icon"
SVG="$ICON_DIR/lumencraft.svg"
SMALL_SVG="$ICON_DIR/lumencraft-small.svg"
APP_ID="org.lumencraft.LumenCraft"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# The SVG is a full-bleed 512 tile, used as-is on Windows and Linux. macOS icons follow Apple's grid:
# the tile is 824/1024 of the canvas with a transparent margin, so widen the viewBox around it.
MAC_SVG="$TMP/mac.svg"
sed 's/viewBox="0 0 512 512"/viewBox="-62.136 -62.136 636.272 636.272"/' "$SVG" >"$MAC_SVG"
grep -q 'viewBox="-62.136' "$MAC_SVG" || { echo "error: $SVG has no 512 viewBox" >&2; exit 1; }

render_svg() {
  local svg="$1" size="$2" out="$3"
  mkdir -p "$(dirname "$out")"
  if command -v resvg >/dev/null; then
    resvg -w "$size" -h "$size" "$svg" "$out" </dev/null
  elif command -v rsvg-convert >/dev/null; then
    rsvg-convert -w "$size" -h "$size" "$svg" -o "$out" </dev/null
  else
    echo "error: need resvg or rsvg-convert to render $svg" >&2
    exit 1
  fi
}

render_svg "$SVG" 1024 "$ICON_DIR/lumencraft-1024.png"
render_svg "$MAC_SVG" 512 "$ICON_DIR/lumencraft-macos-512.png"

for sz in 16 22 24 32 48 64 96 128 256 512; do
  render_svg "$SVG" "$sz" "$ICON_DIR/hicolor/${sz}x${sz}/apps/${APP_ID}.png"
done
mkdir -p "$ICON_DIR/hicolor/scalable/apps"
cp "$SMALL_SVG" "$ICON_DIR/hicolor/scalable/apps/${APP_ID}.svg"

(cd "$ROOT" && cargo xtask ico "$ICON_DIR/lumencraft.ico" \
  "$ICON_DIR/hicolor/16x16/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/24x24/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/32x32/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/48x48/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/64x64/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/128x128/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/256x256/apps/${APP_ID}.png")

if command -v iconutil >/dev/null; then
  ICONSET="$TMP/lumencraft.iconset"
  mkdir -p "$ICONSET"
  for sz in 16 32 128 256 512; do
    render_svg "$MAC_SVG" "$sz" "$ICONSET/icon_${sz}x${sz}.png"
    render_svg "$MAC_SVG" "$((sz * 2))" "$ICONSET/icon_${sz}x${sz}@2x.png"
  done
  iconutil -c icns -o "$ICON_DIR/lumencraft.icns" "$ICONSET"
fi
echo "icons: updated $ICON_DIR"
