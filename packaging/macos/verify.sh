#!/usr/bin/env bash
# Verify macOS release artifacts (.app, .dmg, and CLI .zip):
#   - Strict deep code signature validation (`codesign --verify --strict --deep`)
#   - Notarization ticket & Gatekeeper checks (`xcrun stapler validate`, `spctl`)
#     when signed with a Developer ID certificate or `--require-notarized` is passed.
#
# Usage: packaging/macos/verify.sh [--arch universal|aarch64|x86_64] [--require-notarized] [artifact...]
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

ARCH=universal
REQUIRE_NOTARIZED=0
TARGETS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)
      [[ $# -ge 2 ]] || { echo "error: --arch requires a value" >&2; exit 2; }
      ARCH="$2"
      shift 2
      ;;
    --require-notarized)
      REQUIRE_NOTARIZED=1
      shift
      ;;
    -h | --help)
      sed -n '2,7p' "$0"
      exit 0
      ;;
    *)
      TARGETS+=("$1")
      shift
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: packaging/macos/verify.sh must run on macOS" >&2
  exit 1
fi

if [[ ${#TARGETS[@]} -eq 0 ]]; then
  DMG="$DIST/lumencraft-$VERSION-macos-$ARCH.dmg"
  CLI_ZIP="$DIST/lumencraft-cli-$VERSION-macos-$ARCH.zip"
  TARGETS=("$DMG" "$CLI_ZIP")
  if [[ -d "$ROOT/dist/LumenCraft.app" ]]; then
    TARGETS=("$ROOT/dist/LumenCraft.app" "${TARGETS[@]}")
  fi
fi

WORK="$(mktemp -d)"
MOUNT_POINTS=()
cleanup() {
  local mp
  for mp in "${MOUNT_POINTS[@]+"${MOUNT_POINTS[@]}"}"; do
    hdiutil detach "$mp" -quiet 2>/dev/null || hdiutil detach "$mp" -force -quiet 2>/dev/null || true
  done
  rm -rf "$WORK"
}
trap cleanup EXIT

is_adhoc_signed() {
  local target="$1" info
  info="$(codesign -dv --verbose=4 "$target" 2>&1 || true)"
  if printf '%s\n' "$info" | grep -q '^Signature=adhoc'; then
    return 0
  fi
  if ! printf '%s\n' "$info" | grep -q '^Authority='; then
    return 0
  fi
  return 1
}

verify_app() {
  local app="$1"
  [[ -d "$app" ]] || { echo "error: app bundle not found: $app" >&2; exit 1; }
  echo "==> verifying app signature: $app"
  codesign --verify --strict --deep --verbose=2 "$app"
  if is_adhoc_signed "$app"; then
    if [[ "$REQUIRE_NOTARIZED" -eq 1 ]]; then
      echo "error: $app is ad-hoc signed but --require-notarized was requested" >&2
      exit 1
    fi
    warn "$app is ad-hoc signed; skipping xcrun stapler validate and spctl"
  else
    xcrun stapler validate "$app"
    spctl --assess --type execute -vvv "$app"
  fi
}

verify_dmg() {
  local dmg="$1" mp app_in_dmg
  [[ -f "$dmg" ]] || { echo "error: DMG not found: $dmg" >&2; exit 1; }
  echo "==> verifying DMG signature: $dmg"
  codesign --verify --strict --deep --verbose=2 "$dmg"
  if is_adhoc_signed "$dmg"; then
    if [[ "$REQUIRE_NOTARIZED" -eq 1 ]]; then
      echo "error: $dmg is ad-hoc signed but --require-notarized was requested" >&2
      exit 1
    fi
    warn "$dmg is ad-hoc signed; skipping xcrun stapler validate and spctl"
  else
    xcrun stapler validate "$dmg"
    spctl --assess --type open --context context:primary-signature -vvv "$dmg"
  fi
  mp="$WORK/mount-$(basename "$dmg" .dmg)"
  mkdir -p "$mp"
  hdiutil attach "$dmg" -mountpoint "$mp" -nobrowse -readonly -quiet
  MOUNT_POINTS+=("$mp")
  app_in_dmg="$mp/LumenCraft.app"
  if [[ -d "$app_in_dmg" ]]; then
    verify_app "$app_in_dmg"
  fi
  hdiutil detach "$mp" -quiet
}

verify_cli_zip() {
  local zip_path="$1" stage bin
  [[ -f "$zip_path" ]] || { echo "error: CLI zip not found: $zip_path" >&2; exit 1; }
  echo "==> verifying CLI zip signature: $zip_path"
  stage="$WORK/cli-$(basename "$zip_path" .zip)"
  mkdir -p "$stage"
  ditto -x -k "$zip_path" "$stage"
  bin="$(find "$stage" -type f -name 'lumencraft-cli' | head -n 1)"
  [[ -n "$bin" ]] || { echo "error: lumencraft-cli binary not found inside $zip_path" >&2; exit 1; }
  codesign --verify --strict --deep --verbose=2 "$bin"
  if is_adhoc_signed "$bin" && [[ "$REQUIRE_NOTARIZED" -eq 1 ]]; then
    echo "error: $bin is ad-hoc signed but --require-notarized was requested" >&2
    exit 1
  fi
  "$bin" --version
}

for target in "${TARGETS[@]}"; do
  case "$target" in
    *.app) verify_app "$target" ;;
    *.dmg) verify_dmg "$target" ;;
    *.zip) verify_cli_zip "$target" ;;
    *) echo "error: unsupported macOS artifact type: $target (expected .app, .dmg, or .zip)" >&2; exit 2 ;;
  esac
done

echo "==> macOS verification passed"
