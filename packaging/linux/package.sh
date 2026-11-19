#!/usr/bin/env bash
# Build and package CurveBook for Linux (<arch> is x86_64 or aarch64):
#
#   $DIST/curvebook-<version>-linux-<arch>.AppImage
#
# Usage: packaging/linux/package.sh [--skip-build]
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=org.curvebook.CurveBook

SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,6p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

verify_sha256() { printf '%s  %s\n' "$1" "$2" | sha256sum -c -; }

ARCH="${CROSS_ARCH:-$(uname -m)}"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH (expected x86_64 or aarch64)" >&2; exit 2 ;;
esac
BASENAME="curvebook-$VERSION-linux-$ARCH"

echo "==> CurveBook $VERSION for Linux $ARCH (AppImage)"

if [[ "$SKIP_BUILD" = 0 ]]; then
  if [[ -n "${CROSS_TARGET:-}" ]]; then
    (cd "$ROOT" && cargo build --release --locked -p curvebook -p curvebook-cli --target "$CROSS_TARGET")
  else
    (cd "$ROOT" && cargo build --release --locked -p curvebook -p curvebook-cli)
  fi
fi
BIN="$CARGO_TARGET_DIR/${CROSS_TARGET:+$CROSS_TARGET/}release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

install -Dm755 "$BIN/curvebook" "$STAGE/usr/bin/curvebook"
install -Dm755 "$BIN/curvebook-cli" "$STAGE/usr/bin/curvebook-cli"
"${CROSS_COMPILE:-}strip" "$STAGE/usr/bin/curvebook" "$STAGE/usr/bin/curvebook-cli" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$CURVEBOOK_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/curvebook"
copy_docs "$STAGE/usr/share/doc/curvebook"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

APPIMAGETOOL_VERSION=1.9.1
case "$ARCH" in
  x86_64) APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 ;;
  aarch64) APPIMAGETOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158 ;;
esac
APPIMAGE_RUNTIME_VERSION=20251108
case "$ARCH" in
  x86_64) APPIMAGE_RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
  aarch64) APPIMAGE_RUNTIME_SHA256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
esac

APPDIR="$WORK/CurveBook.AppDir"
cp -R "$STAGE" "$APPDIR"
ln -s usr/bin/curvebook "$APPDIR/AppRun"
cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
cp "$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
if [[ -z "$TOOL" ]]; then
  TOOL="$CARGO_TARGET_DIR/appimagetool-$APPIMAGETOOL_VERSION-$ARCH.AppImage"
  if [[ ! -x "$TOOL" ]]; then
    download="$TOOL.download"
    curl --proto '=https' -fsSL -o "$download" \
      "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$ARCH.AppImage"
    verify_sha256 "$APPIMAGETOOL_SHA256" "$download"
    chmod 755 "$download"
    mv "$download" "$TOOL"
  fi
  verify_sha256 "$APPIMAGETOOL_SHA256" "$TOOL"
fi
RUNTIME="$CARGO_TARGET_DIR/appimage-runtime-$APPIMAGE_RUNTIME_VERSION-$ARCH"
if [[ ! -f "$RUNTIME" ]]; then
  download="$RUNTIME.download"
  curl --proto '=https' -fsSL -o "$download" \
    "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-$ARCH"
  verify_sha256 "$APPIMAGE_RUNTIME_SHA256" "$download"
  mv "$download" "$RUNTIME"
fi
verify_sha256 "$APPIMAGE_RUNTIME_SHA256" "$RUNTIME"

OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
APPDIR="$(cd "$APPDIR" && pwd)"
TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
RUNTIME="$(cd "$(dirname "$RUNTIME")" && pwd)/$(basename "$RUNTIME")"
rm -f "$OUT"
(cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream --runtime-file "$RUNTIME" "$APPDIR" "$OUT")
echo "wrote $OUT"

if [[ -z "${CROSS_TARGET:-}" ]]; then
  "$STAGE/usr/bin/curvebook-cli" --version
elif [[ -n "${EMULATOR:-}" ]]; then
  "$EMULATOR" "$STAGE/usr/bin/curvebook-cli" --version
fi
echo "==> done"
ls -lh "$OUT"
