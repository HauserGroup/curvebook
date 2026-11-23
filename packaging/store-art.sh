#!/usr/bin/env bash
# Regenerate Microsoft Store listing images in assets/app-icon/store/ from the
# approved seven-pane artwork (assets/app-icon/source/approved-seven-pane.png).
#
#   poster-<w>x<h>.png  9:16 poster art (opaque, with wordmark)
#   box-<n>x<n>.png     1:1 box art (opaque, with wordmark)
#   hero-<w>x<h>.png    16:9 super hero art (opaque, no title by Store rule)
#   tile-<n>x<n>.png    1:1 Store display logos (transparent, artwork only)
#
# Requires ImageMagick 7 (`magick`) with FreeType. The wordmark is rasterised
# from a system font (no font file is bundled); override it with
# CURVEBOOK_STORE_FONT=/path/to/font.
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

command -v magick >/dev/null || { echo "error: need ImageMagick 7 (magick)" >&2; exit 1; }

ICON_DIR="$ROOT/assets/app-icon"
SRC="$ICON_DIR/source/approved-seven-pane.png"
OUT="$ICON_DIR/store"
FONT="${CURVEBOOK_STORE_FONT:-/System/Library/Fonts/Avenir Next.ttc}"
[ -f "$FONT" ] || { echo "error: font not found: $FONT (set CURVEBOOK_STORE_FONT)" >&2; exit 1; }

BG_CENTRE="#2a3442"
BG_EDGE="#090c11"
INK="#f3f5f8"

mkdir -p "$ROOT/target" "$OUT"
TMP="$(mktemp -d "$ROOT/target/store-art.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# Artwork trimmed to its visible pixels (including the soft glow).
ART="$TMP/art.png"
magick "$SRC" -fuzz 1% -trim +repage "PNG32:$ART"

# compose W H ART_WIDTH ART_CENTRE_Y TEXT_SIZE TEXT_BASELINE_Y OUT
compose() {
  local w="$1" h="$2" aw="$3" acy="$4" ts="$5" ty="$6" out="$7"
  local art_scaled="$TMP/art-$w-$h.png" ah ay
  magick "$ART" -filter Lanczos -resize "${aw}x" "PNG32:$art_scaled"
  ah="$(magick identify -format '%h' "$art_scaled")"
  ay=$((acy - ah / 2))
  magick -size "${w}x${h}" "radial-gradient:${BG_CENTRE}-${BG_EDGE}" \
    "$art_scaled" -gravity north -geometry "+0+${ay}" -composite \
    -font "$FONT" -pointsize "$ts" -fill "$INK" -kerning -2 \
    -gravity north -annotate "+0+${ty}" "CurveBook" \
    -alpha off -strip "PNG24:$out"
}

# Large masters; the 1x sizes are Lanczos downsamples of the 2x masters.
compose 1440 2160 1180 900 168 1500 "$OUT/poster-1440x2160.png"
compose 2160 2160 1560 930 200 1680 "$OUT/box-2160x2160.png"
magick "$OUT/poster-1440x2160.png" -filter Lanczos -resize 720x1080 -strip "PNG24:$OUT/poster-720x1080.png"
magick "$OUT/box-2160x2160.png" -filter Lanczos -resize 1080x1080 -strip "PNG24:$OUT/box-1080x1080.png"

# 16:9 super hero art: no title (the Store overlays it bottom-left), so the
# artwork sits right of centre and high, over a faint plotting grid.
hero() {
  local w=3840 h=2160 aw=2000 acx=2400 acy=900 step=160
  local art_scaled="$TMP/art-hero.png" grid="$TMP/grid.png" aw2 ah ax ay x y draw=""
  magick "$ART" -filter Lanczos -resize "${aw}x" "PNG32:$art_scaled"
  aw2="$(magick identify -format '%w' "$art_scaled")"
  ah="$(magick identify -format '%h' "$art_scaled")"
  ax=$((acx - aw2 / 2)); ay=$((acy - ah / 2))
  for ((x = step; x < w; x += step)); do draw+="line $x,0 $x,$h "; done
  for ((y = step; y < h; y += step)); do draw+="line 0,$y $w,$y "; done
  # Dim grid lines on black, faded towards the edges, then screen-blended
  # onto the background so they only ever lighten it slightly.
  magick -size "${w}x${h}" xc:black -stroke "#2c3644" -strokewidth 2 -draw "$draw" \
    \( -size "${w}x${h}" radial-gradient:white-black \) \
    -compose multiply -composite -alpha off "PNG24:$grid"
  magick -size "${w}x${h}" "radial-gradient:${BG_CENTRE}-${BG_EDGE}" \
    "$grid" -compose screen -composite \
    "$art_scaled" -compose over -geometry "+${ax}+${ay}" -composite \
    -alpha off -strip "PNG24:$OUT/hero-3840x2160.png"
  magick "$OUT/hero-3840x2160.png" -filter Lanczos -resize 1920x1080 -strip "PNG24:$OUT/hero-1920x1080.png"
}
hero

# Transparent square logos: artwork fitted within a 94% safe area.
for sz in 300 150 71; do
  inner=$((sz * 94 / 100))
  magick "$ART" -filter Lanczos -resize "${inner}x${inner}" \
    -background none -gravity center -extent "${sz}x${sz}" \
    -strip "PNG32:$OUT/tile-${sz}x${sz}.png"
done

echo "store-art: updated $OUT"
