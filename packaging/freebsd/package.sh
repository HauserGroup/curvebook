#!/usr/bin/env bash
# Build and package CurveBook for FreeBSD (Tier 2: headless CLI + MCP):
#
#   $DIST/curvebook-<version>-freebsd-<arch>.tar.gz
#
# Usage: packaging/freebsd/package.sh [--skip-build]
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,6p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

case "$(uname -m)" in
  amd64 | x86_64) HOST_ARCH=x86_64 ;;
  arm64 | aarch64) HOST_ARCH=aarch64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 2 ;;
esac
ARCH="$HOST_ARCH"
BIN="$CARGO_TARGET_DIR/release"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  case "$CARGO_BUILD_TARGET" in
    aarch64-unknown-freebsd) ARCH=aarch64 ;;
    *) ARCH=x86_64 ;;
  esac
  BIN="$CARGO_TARGET_DIR/$CARGO_BUILD_TARGET/release"
fi
BASENAME="curvebook-$VERSION-freebsd-$ARCH"

echo "==> CurveBook $VERSION for FreeBSD $ARCH"

if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p curvebook-cli ${CARGO_BUILD_TARGET:+--target "$CARGO_BUILD_TARGET"})
fi
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$BASENAME"
rm -rf "$WORK"

mkdir -p "$STAGE/bin" "$STAGE/share/doc/curvebook"
install -m 755 "$BIN/curvebook-cli" "$STAGE/bin/"
strip "$STAGE/bin/curvebook-cli" 2>/dev/null || true
copy_docs "$STAGE/share/doc/curvebook"

tar -C "$WORK" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $DIST/$BASENAME.tar.gz"

if [[ "$ARCH" = "$HOST_ARCH" ]]; then "$STAGE/bin/curvebook-cli" --version; fi
echo "==> done"
ls -lh "$DIST"
