# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
#
# Exports:
#   ROOT                    workspace root
#   VERSION                 [workspace.package] version from Cargo.toml (override: CURVEBOOK_VERSION)
#   DIST                    output directory for release artifacts (default: $ROOT/dist/release)
#   CURVEBOOK_BUILD_SHA     git commit baked into the binaries
#   CURVEBOOK_BUILD_DATE    UTC build date, YYYY-MM-DD
#   CARGO_TARGET_DIR        cargo's target dir (default: $ROOT/target)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

workspace_version() {
  awk '
    {
      # Strip # comments outside quotes cleanly
      out = ""
      in_dq = 0
      in_sq = 0
      esc = 0
      for (i = 1; i <= length($0); i++) {
        c = substr($0, i, 1)
        if (in_dq) {
          if (esc) { esc = 0 }
          else if (c == "\\") { esc = 1 }
          else if (c == "\"") { in_dq = 0 }
        } else if (in_sq) {
          if (c == "'"'"'") { in_sq = 0 }
        } else {
          if (c == "\"") { in_dq = 1 }
          else if (c == "'"'"'") { in_sq = 1 }
          else if (c == "#") { break }
        }
        out = out c
      }
      sub(/^[[:space:]]+/, "", out)
      sub(/[[:space:]]+$/, "", out)
      if (out == "") next
      if (substr(out, 1, 1) == "[") {
        in_pkg = (out == "[workspace.package]")
        next
      }
      if (in_pkg && out ~ /^version[[:space:]]*=/) {
        sub(/^version[[:space:]]*=[[:space:]]*/, "", out)
        gsub(/^["'"'"']|["'"'"']$/, "", out)
        print out
        exit
      }
    }
  ' "$ROOT/Cargo.toml"
}

VERSION="${CURVEBOOK_VERSION:-$(workspace_version)}"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

if [[ -z "${CURVEBOOK_BUILD_SHA:-}" ]]; then
  CURVEBOOK_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export CURVEBOOK_BUILD_SHA
export CURVEBOOK_BUILD_DATE="${CURVEBOOK_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE COPYRIGHT NOTICE ATTRIBUTION.md ASSETS.md; do
    if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
  done
  if [[ -d "$ROOT/licenses" ]]; then
    mkdir -p "$dest/licenses"
    cp -R "$ROOT/licenses/." "$dest/licenses/"
  fi
  copy_font_licences "$dest"
}

copy_font_licences() {
  local dest="$1" ofl family
  [[ -n "${CURVEBOOK_FONTS_DIR:-}" ]] || return 0
  for ofl in "$CURVEBOOK_FONTS_DIR"/fonts/*/OFL.txt; do
    [[ -f "$ofl" ]] || continue
    family="$(basename "$(dirname "$ofl")")"
    cp "$ofl" "$dest/OFL-$family.txt"
  done
}

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
