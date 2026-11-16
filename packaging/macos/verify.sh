#!/usr/bin/env bash
# Verify macOS release artifacts (.app, .dmg, and CLI .zip):
#   - Strict deep code signature validation (`codesign --verify --strict --deep`)
#   - Notarization & Gatekeeper checks (`xcrun stapler validate`, `spctl`, `codesign --test-requirement="=notarized"`)
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
  DMG="$DIST/curvebook-$VERSION-macos-$ARCH.dmg"
  CLI_ZIP="$DIST/curvebook-cli-$VERSION-macos-$ARCH.zip"
  TARGETS=("$DMG" "$CLI_ZIP")
  if [[ -d "$ROOT/dist/CurveBook.app" ]]; then
    TARGETS=("$ROOT/dist/CurveBook.app" "${TARGETS[@]}")
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
  app_in_dmg="$mp/CurveBook.app"
  if [[ ! -d "$app_in_dmg" ]]; then
    echo "error: CurveBook.app not found inside $dmg (expected $app_in_dmg)" >&2
    exit 1
  fi
  verify_app "$app_in_dmg"
  hdiutil detach "$mp" -quiet
}

verify_cli_zip() {
  local zip_path="$1" stage bin cs_info host_arch bin_archs can_run
  [[ -f "$zip_path" ]] || { echo "error: CLI zip not found: $zip_path" >&2; exit 1; }
  echo "==> verifying CLI zip signature: $zip_path"
  stage="$WORK/cli-$(basename "$zip_path" .zip)"
  mkdir -p "$stage"
  ditto -x -k "$zip_path" "$stage"
  bin="$(find "$stage" -type f -name 'curvebook-cli' | head -n 1)"
  [[ -n "$bin" ]] || { echo "error: curvebook-cli binary not found inside $zip_path" >&2; exit 1; }
  codesign --verify --strict --deep --verbose=2 "$bin"
  if is_adhoc_signed "$bin"; then
    if [[ "$REQUIRE_NOTARIZED" -eq 1 ]]; then
      echo "error: $bin is ad-hoc signed but --require-notarized was requested" >&2
      exit 1
    fi
    warn "$bin is ad-hoc signed; skipping Gatekeeper and hardened-runtime notarization checks"
  else
    cs_info="$(codesign -dv --verbose=4 "$bin" 2>&1 || true)"
    if ! printf '%s\n' "$cs_info" | grep -q 'flags=.*runtime'; then
      echo "error: $bin lacks Hardened Runtime flag required for notarization" >&2
      exit 1
    fi
    if ! printf '%s\n' "$cs_info" | grep -q '^Timestamp='; then
      echo "error: $bin lacks a secure signing timestamp required for notarization" >&2
      exit 1
    fi
    codesign --test-requirement="=notarized" --verify --strict --verbose=2 "$bin"
  fi
  host_arch="$(uname -m)"
  bin_archs="$(lipo -archs "$bin" 2>/dev/null || true)"
  can_run=0
  case " $bin_archs " in
    *" $host_arch "*) can_run=1 ;;
    *" x86_64 "*)
      if [[ "$host_arch" == "arm64" ]] && arch -x86_64 /usr/bin/true >/dev/null 2>&1; then
        can_run=1
      fi
      ;;
  esac
  if [[ "$can_run" -eq 1 ]]; then
    "$bin" --version
  else
    warn "skipping $bin --version: binary architectures ($bin_archs) incompatible with host $host_arch"
  fi
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
