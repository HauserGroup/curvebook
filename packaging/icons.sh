#!/usr/bin/env bash
# Regenerate rasterized/platform icons in assets/app-icon/ from curvebook.svg and curvebook-small.svg.
# Requires: resvg, rsvg-convert, or magick; cargo (for xtask ico); and iconutil on macOS (for .icns).
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

ICON_DIR="$ROOT/assets/app-icon"
SVG="$ICON_DIR/curvebook.svg"
SMALL_SVG="$ICON_DIR/curvebook-small.svg"
APP_ID="org.curvebook.CurveBook"
mkdir -p "$ROOT/target"
TMP="$(mktemp -d "$ROOT/target/app-icons.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# The SVG has a transparent 512-square canvas, used as-is on Windows and Linux.
# macOS adds the existing 824/1024 icon-grid inset by widening the viewBox.
MAC_SVG="$TMP/mac.svg"
sed 's/viewBox="0 0 512 512"/viewBox="-62.136 -62.136 636.272 636.272"/' "$SVG" >"$MAC_SVG"
grep -q 'viewBox="-62.136' "$MAC_SVG" || { echo "error: $SVG has no 512 viewBox" >&2; exit 1; }

render_svg() {
  local svg="$1" size="$2" out="$3" density
  mkdir -p "$(dirname "$out")"
  if command -v resvg >/dev/null; then
    resvg -w "$size" -h "$size" "$svg" "$out" </dev/null
  elif command -v rsvg-convert >/dev/null; then
    rsvg-convert -w "$size" -h "$size" "$svg" -o "$out" </dev/null
  elif command -v magick >/dev/null; then
    density="$(awk -v size="$size" 'BEGIN { print size * 96 / 512 }')"
    magick -background none -density "$density" "MSVG:$svg" -resize "${size}x${size}" "PNG32:$out" </dev/null
  else
    echo "error: need resvg, rsvg-convert, or magick to render $svg" >&2
    exit 1
  fi
}

render_svg "$SVG" 1024 "$ICON_DIR/curvebook-1024.png"
render_svg "$MAC_SVG" 512 "$ICON_DIR/curvebook-macos-512.png"

for sz in 16 22 24 32 48 64 96 128 256 512; do
  render_svg "$SVG" "$sz" "$ICON_DIR/hicolor/${sz}x${sz}/apps/${APP_ID}.png"
done
mkdir -p "$ICON_DIR/hicolor/scalable/apps"
cp "$SMALL_SVG" "$ICON_DIR/hicolor/scalable/apps/${APP_ID}.svg"

(cd "$ROOT" && cargo run --locked -p xtask -- ico "$ICON_DIR/curvebook.ico" \
  "$ICON_DIR/hicolor/16x16/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/24x24/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/32x32/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/48x48/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/64x64/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/128x128/apps/${APP_ID}.png" \
  "$ICON_DIR/hicolor/256x256/apps/${APP_ID}.png")

if command -v iconutil >/dev/null; then
  ICONSET="$TMP/curvebook.iconset"
  mkdir -p "$ICONSET"
  for sz in 16 32 128 256 512; do
    render_svg "$MAC_SVG" "$sz" "$ICONSET/icon_${sz}x${sz}.png"
    render_svg "$MAC_SVG" "$((sz * 2))" "$ICONSET/icon_${sz}x${sz}@2x.png"
  done
  iconutil -c icns -o "$ICON_DIR/curvebook.icns" "$ICONSET"
fi
echo "icons: updated $ICON_DIR"
