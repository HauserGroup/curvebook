#!/usr/bin/env bash
# Verify macOS release artifacts (.app and .dmg, including bundled curvebook-cli):
#   - Strict deep code signature validation (`codesign --verify --strict --deep`)
#   - Notarization & Gatekeeper checks (`xcrun stapler validate`, `spctl`, `codesign --test-requirement="=notarized"`)
#     when signed with a Developer ID certificate or `--require-notarized` is passed.
#
# Environment overrides:
#   CURVEBOOK_VERIFY_RETRIES           Bounded retry attempts for network-backed notarization checks (default: 5)
#   CURVEBOOK_VERIFY_RETRY_DELAY_SECS  Fixed delay in seconds between failed retry attempts (default: 10)
#   CURVEBOOK_COMPATIBILITY_FIXTURE_DIR  Optional new directory for a compatibility capture from the mounted DMG
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
      sed -n '2,11p' "$0"
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

VERIFY_RETRIES="${CURVEBOOK_VERIFY_RETRIES:-5}"
VERIFY_RETRY_DELAY_SECS="${CURVEBOOK_VERIFY_RETRY_DELAY_SECS:-10}"
if [[ ! "$VERIFY_RETRIES" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: CURVEBOOK_VERIFY_RETRIES must be a positive integer (got '$VERIFY_RETRIES')" >&2
  exit 2
fi
if [[ ! "$VERIFY_RETRY_DELAY_SECS" =~ ^[0-9]+$ ]]; then
  echo "error: CURVEBOOK_VERIFY_RETRY_DELAY_SECS must be a non-negative integer (got '$VERIFY_RETRY_DELAY_SECS')" >&2
  exit 2
fi

if [[ ${#TARGETS[@]} -eq 0 ]]; then
  DMG="$DIST/curvebook-$VERSION-macos-$ARCH.dmg"
  TARGETS=("$DMG")
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

# Retry transient network-backed Gatekeeper/notarization checks only.
# Never wrap local cryptographic `codesign --verify --strict` checks with this helper.
retry_notarization_check() {
  if [[ $# -eq 0 ]]; then
    echo "error: retry_notarization_check requires a command" >&2
    return 2
  fi
  local retries="$VERIFY_RETRIES"
  local delay="$VERIFY_RETRY_DELAY_SECS"
  local attempt=1
  local rc=1
  local cmd_str
  printf -v cmd_str '%q ' "$@"
  cmd_str="${cmd_str% }"

  while [[ "$attempt" -le "$retries" ]]; do
    if "$@"; then
      return 0
    else
      rc=$?
    fi
    if [[ "$attempt" -lt "$retries" ]]; then
      echo "warning: notarization check failed (attempt $attempt/$retries, exit $rc): $cmd_str; retrying in ${delay}s" >&2
      if [[ "$delay" -gt 0 ]]; then
        sleep "$delay"
      fi
    else
      echo "error: notarization check failed (attempt $attempt/$retries, exit $rc): $cmd_str" >&2
    fi
    attempt=$((attempt + 1))
  done
  return "$rc"
}

verify_bundled_cli() {
  local app="$1" bin cs_info host_arch bin_archs can_run
  bin="$app/Contents/MacOS/curvebook-cli"
  [[ -x "$bin" ]] || { echo "error: bundled curvebook-cli not found or not executable: $bin" >&2; exit 1; }
  echo "==> verifying bundled CLI signature: $bin"
  codesign --verify --strict --verbose=2 "$bin"
  if is_adhoc_signed "$bin"; then
    if [[ "$REQUIRE_NOTARIZED" -eq 1 ]]; then
      echo "error: $bin is ad-hoc signed but --require-notarized was requested" >&2
      exit 1
    fi
    warn "$bin is ad-hoc signed; skipping hardened-runtime and notarization checks"
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
    retry_notarization_check codesign --test-requirement="=notarized" --verify --strict --verbose=2 "$bin"
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
    retry_notarization_check xcrun stapler validate "$app"
    retry_notarization_check spctl --assess --type execute -vvv "$app"
  fi
  verify_bundled_cli "$app"
}

capture_compatibility_fixture() {
  local bin="$1" dmg="$2" destination="$3"
  local fixture="$WORK/compatibility-fixture" writer installer_digest
  if [[ -e "$destination" || -L "$destination" ]]; then
    echo "error: compatibility fixture destination already exists: $destination" >&2
    exit 1
  fi
  mkdir "$fixture"
  cp "$ROOT/apps/curvebook-cli/tests/fixtures/release_compatibility/batch.json" "$fixture/batch.json"
  "$bin" batch "$fixture/batch.json" --workdir "$fixture"
  "$bin" info "$fixture/release-compatibility.curvebook" >/dev/null
  writer="$("$bin" --version)"
  installer_digest="$(shasum -a 256 < "$dmg")"
  printf '%s\nDMG SHA-256: %s\n' "$writer" "${installer_digest%% *}" > "$fixture/writer.txt"
  mv "$fixture" "$destination"
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
    retry_notarization_check xcrun stapler validate "$dmg"
    retry_notarization_check spctl --assess --type open --context context:primary-signature -vvv "$dmg"
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
  if [[ -n "${CURVEBOOK_COMPATIBILITY_FIXTURE_DIR:-}" ]]; then
    capture_compatibility_fixture "$app_in_dmg/Contents/MacOS/curvebook-cli" "$dmg" "$CURVEBOOK_COMPATIBILITY_FIXTURE_DIR"
  fi
  hdiutil detach "$mp" -quiet
}

for target in "${TARGETS[@]}"; do
  case "$target" in
    *.app) verify_app "$target" ;;
    *.dmg) verify_dmg "$target" ;;
    *) echo "error: unsupported macOS artifact type: $target (expected .app or .dmg)" >&2; exit 2 ;;
  esac
done

echo "==> macOS verification passed"
