#!/usr/bin/env bash
# Build the browser version and zip it: $DIST/curvebook-web-<version>.zip
#
# Usage: packaging/web/package.sh [--skip-build]
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/web"

export CARGO_NET_LOCKED=true
if [[ "${1:-}" != "--skip-build" ]]; then
  command -v trunk >/dev/null || { echo "error: trunk not found (cargo install trunk --locked)" >&2; exit 1; }
  (cd "$ROOT/apps/curvebook-web" && trunk build --release --cargo-profile web-release)
fi

SITE="$ROOT/dist/web"
[[ -f "$SITE/index.html" ]] || { echo "error: $SITE/index.html missing; run without --skip-build" >&2; exit 1; }
if grep -Eq '(src|href)="/[^/]' "$SITE/index.html"; then
  echo "error: $SITE/index.html has root-absolute URLs; it would break when served from a sub-path" >&2
  exit 1
fi

NAME="curvebook-web-$VERSION"
WORK="$CARGO_TARGET_DIR/web-package"
rm -rf "$WORK"
mkdir -p "$WORK/$NAME"
cp -R "$SITE/." "$WORK/$NAME/"
cp "$HERE/_headers" "$HERE/.htaccess" "$WORK/$NAME/"

command -v python3 >/dev/null || { echo "error: python3 is required to stamp SHA-256 script CSP hashes" >&2; exit 1; }
python3 - "$WORK/$NAME" <<'PY'
import base64
import hashlib
import pathlib
import re
import sys

stage = pathlib.Path(sys.argv[1])
index_path = stage / "index.html"
html = index_path.read_text(encoding="utf-8")

hashes = []
pattern = re.compile(r"<script\b([^>]*)>(.*?)</script\s*>", re.IGNORECASE | re.DOTALL)
for match in pattern.finditer(html):
    attrs = match.group(1)
    if attrs.rstrip().endswith("/"):
        continue
    if re.search(r"(?:^|\s)src\s*=", attrs, re.IGNORECASE):
        continue
    digest = hashlib.sha256(match.group(2).encode("utf-8")).digest()
    token = f"'sha256-{base64.b64encode(digest).decode('ascii')}'"
    if token not in hashes:
        hashes.append(token)

script_src = " ".join(["script-src", "'self'", *hashes, "'wasm-unsafe-eval'"])

for target in [index_path, stage / "_headers", stage / ".htaccess"]:
    text = target.read_text(encoding="utf-8")
    updated = re.sub(r"script-src\b[^;\"\r\n]*", script_src, text)
    if "'unsafe-inline'" in updated:
        raise SystemExit(f"error: 'unsafe-inline' still present in packaged {target}")
    target.write_text(updated, encoding="utf-8")
PY

cp "$HERE/README.md" "$WORK/$NAME/HOSTING.md"
copy_docs "$WORK/$NAME"
rm -f "$DIST/$NAME.zip"
(cd "$WORK" && zip -qr9 "$DIST/$NAME.zip" "$NAME")
echo "wrote $DIST/$NAME.zip"
ls -lh "$DIST/$NAME.zip"
