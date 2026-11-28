#!/usr/bin/env bash
# shellcheck shell=bash
# Generate a deterministic, path-normalized CycloneDX 1.5 JSON SBOM for CurveBook
# covering both shipped binaries (`curvebook` desktop and `curvebook-cli` / MCP)
# across all Cargo targets (`R14.2`).
#
# Usage: packaging/sbom.sh [output-dir]
#
# Writes `<output-dir>/curvebook-<version>.cdx.json` (default `<repo>/sbom/`).
# Requires `cargo-cyclonedx` (CI-only tool, never a workspace dependency) and `python3`.
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  sed -n '3,10p' "$0"
  exit 0
fi
if [[ $# -gt 1 ]]; then
  echo "usage: packaging/sbom.sh [output-dir]" >&2
  exit 2
fi

OUT_DIR="${1:-$ROOT/sbom}"
mkdir -p "$OUT_DIR"
OUT_PATH="$OUT_DIR/curvebook-$VERSION.cdx.json"

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  SOURCE_DATE_EPOCH="$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || true)"
fi
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  echo "error: SOURCE_DATE_EPOCH is unset and git commit timestamp could not be read" >&2
  exit 1
fi
export SOURCE_DATE_EPOCH

TMP_STEM=".curvebook-sbom-tmp-$$.cdx"
DESKTOP_RAW="$ROOT/apps/curvebook/$TMP_STEM.json"
CLI_RAW="$ROOT/apps/curvebook-cli/$TMP_STEM.json"
TMP_OUT="$OUT_PATH.tmp.$$"

cleanup() {
  local dir
  rm -f "$TMP_OUT" "$ROOT/$TMP_STEM.json"
  for dir in "$ROOT/apps" "$ROOT/crates" "$ROOT/xtask"; do
    if [[ -d "$dir" ]]; then
      find "$dir" -maxdepth 2 -name "$TMP_STEM.json" -delete
    fi
  done
}
trap cleanup EXIT

# `cargo cyclonedx` emits one BOM file adjacent to every workspace member's Cargo.toml.
# Run once across the workspace, merge the two shipped application graphs (`curvebook` and
# `curvebook-cli`), normalize local workspace paths, and remove all per-member temp files on exit.
cargo cyclonedx \
  --manifest-path "$ROOT/Cargo.toml" \
  --format json \
  --spec-version 1.5 \
  --target all \
  --override-filename "$TMP_STEM"

if [[ ! -s "$DESKTOP_RAW" || ! -s "$CLI_RAW" ]]; then
  echo "error: cargo cyclonedx did not produce expected BOMs at $DESKTOP_RAW and $CLI_RAW" >&2
  exit 1
fi

python3 - "$ROOT" "$DESKTOP_RAW" "$CLI_RAW" "$TMP_OUT" <<'PYEOF'
import hashlib
import json
import pathlib
import sys
import uuid

ws_root = pathlib.Path(sys.argv[1]).resolve().as_posix().rstrip("/")
desktop_path = pathlib.Path(sys.argv[2])
cli_path = pathlib.Path(sys.argv[3])
out_path = pathlib.Path(sys.argv[4])

file_prefixes = (
    f"path+file://{ws_root}/",
    f"file://{ws_root}/",
)


def normalize(obj):
    if isinstance(obj, str):
        for prefix in file_prefixes:
            if obj.startswith(prefix):
                return "workspace:" + obj[len(prefix):]
        if obj.startswith("pkg:cargo/") and "?download_url=file://" in obj:
            return obj.split("?download_url=file://", 1)[0]
        return obj
    if isinstance(obj, list):
        return [normalize(item) for item in obj]
    if isinstance(obj, dict):
        return {k: normalize(v) for k, v in obj.items()}
    return obj


desktop = normalize(json.loads(desktop_path.read_text(encoding="utf-8")))
cli = normalize(json.loads(cli_path.read_text(encoding="utf-8")))

components_by_ref = {}
for comp in desktop.get("components", []):
    components_by_ref[comp["bom-ref"]] = comp

cli_root = cli.get("metadata", {}).get("component")
if isinstance(cli_root, dict) and "bom-ref" in cli_root:
    components_by_ref.setdefault(cli_root["bom-ref"], cli_root)

for comp in cli.get("components", []):
    components_by_ref.setdefault(comp["bom-ref"], comp)

deps_by_ref = {}
for bom in (desktop, cli):
    for dep in bom.get("dependencies", []):
        ref = dep["ref"]
        deps_by_ref.setdefault(ref, set()).update(dep.get("dependsOn", []))

root_ref = desktop["metadata"]["component"]["bom-ref"]
if isinstance(cli_root, dict) and "bom-ref" in cli_root:
    deps_by_ref.setdefault(root_ref, set()).add(cli_root["bom-ref"])

merged_body = {
    "bomFormat": desktop["bomFormat"],
    "specVersion": desktop["specVersion"],
    "version": desktop.get("version", 1),
    "metadata": desktop["metadata"],
    "components": sorted(
        components_by_ref.values(),
        key=lambda c: (c.get("name", ""), c.get("version", ""), c.get("bom-ref", "")),
    ),
    "dependencies": [
        {"ref": ref, "dependsOn": sorted(deps)}
        for ref, deps in sorted(deps_by_ref.items())
    ],
}

# Derive a deterministic RFC 4122 UUIDv5 `urn:uuid:...` serialNumber from the normalized BOM
# payload so reproducible rebuilds stay byte-identical while satisfying CycloneDX 1.5 and
# `actions/attest` (`bomFormat` + `serialNumber` + `specVersion`).
body_digest = hashlib.sha256(
    json.dumps(merged_body, sort_keys=True, separators=(",", ":")).encode("utf-8")
).hexdigest()
serial_number = f"urn:uuid:{uuid.uuid5(uuid.NAMESPACE_URL, f'https://github.com/HauserGroup/curvebook/sbom/{body_digest}')}"

merged = {}
if isinstance(desktop.get("$schema"), str) and desktop["$schema"].strip():
    merged["$schema"] = desktop["$schema"]
merged["bomFormat"] = merged_body["bomFormat"]
merged["specVersion"] = merged_body["specVersion"]
merged["serialNumber"] = serial_number
merged["version"] = merged_body["version"]
merged["metadata"] = merged_body["metadata"]
merged["components"] = merged_body["components"]
merged["dependencies"] = merged_body["dependencies"]

payload = json.dumps(merged, indent=2) + "\n"
out_path.write_text(payload, encoding="utf-8")
PYEOF

mv -f "$TMP_OUT" "$OUT_PATH"
test -s "$OUT_PATH"
echo "sbom: generated $OUT_PATH"
