#!/usr/bin/env bash
# Import a base64-encoded Developer ID Application .p12 into an ephemeral keychain for CI signing.
# Expects: APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD, KEYCHAIN_PASSWORD.
set -euo pipefail

: "${APPLE_CERTIFICATE:?APPLE_CERTIFICATE secret missing}"
: "${APPLE_CERTIFICATE_PASSWORD:?APPLE_CERTIFICATE_PASSWORD secret missing}"
: "${KEYCHAIN_PASSWORD:?KEYCHAIN_PASSWORD secret missing}"

CERT_P12="${RUNNER_TEMP:-/tmp}/curvebook-signing.p12"
KEYCHAIN="${RUNNER_TEMP:-/tmp}/curvebook-signing.keychain-db"
rm -f "$CERT_P12" "$KEYCHAIN"

printf '%s' "$APPLE_CERTIFICATE" | base64 --decode >"$CERT_P12"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security import "$CERT_P12" -k "$KEYCHAIN" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security list-keychains -d user -s "$KEYCHAIN" login.keychain-db
rm -f "$CERT_P12"

IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" | awk -F '"' '/Developer ID Application/ {print $2; exit}')"
if [[ -z "$IDENTITY" ]]; then
  echo "error: no Developer ID Application identity found in imported certificate" >&2
  exit 1
fi
if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    echo "MACOS_SIGN_IDENTITY=$IDENTITY"
    echo "MACOS_KEYCHAIN=$KEYCHAIN"
  } >>"$GITHUB_ENV"
fi
echo "imported signing identity: $IDENTITY"
