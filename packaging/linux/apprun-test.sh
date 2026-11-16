#!/usr/bin/env bash
# Extract a Linux AppImage via --appimage-extract and smoke-run --version and --help.
#
# Usage: packaging/linux/apprun-test.sh [path/to/CurveBook.AppImage]
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

case "${1:-}" in
  -h | --help)
    sed -n '2,4p' "$0"
    exit 0
    ;;
esac

ARCH="${CROSS_ARCH:-$(uname -m)}"
case "$ARCH" in
  arm64) ARCH=aarch64 ;;
esac

APPIMAGE="${1:-$DIST/curvebook-$VERSION-linux-$ARCH.AppImage}"
[[ -f "$APPIMAGE" ]] || { echo "error: AppImage not found: $APPIMAGE" >&2; exit 1; }

APPIMAGE_ABS="$(cd "$(dirname "$APPIMAGE")" && pwd)/$(basename "$APPIMAGE")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cp "$APPIMAGE_ABS" "$WORK/CurveBook.AppImage"
chmod +x "$WORK/CurveBook.AppImage"

echo "==> extracting $APPIMAGE"
(cd "$WORK" && ./CurveBook.AppImage --appimage-extract >/dev/null)

APPRUN="$WORK/squashfs-root/AppRun"
[[ -x "$APPRUN" ]] || { echo "error: extracted AppRun missing or not executable: $APPRUN" >&2; exit 1; }

echo "==> smoke-testing AppRun --version and --help"
"$APPRUN" --version
"$APPRUN" --help >/dev/null

CLI_BIN="$WORK/squashfs-root/usr/bin/curvebook-cli"
if [[ -x "$CLI_BIN" ]]; then
  "$CLI_BIN" --version
  "$CLI_BIN" --help >/dev/null
fi

echo "==> AppImage AppRun smoke test passed"
