#!/usr/bin/env bash
# App-specific package assertions retained alongside shared packaging lint.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

python3 - <<'PYEOF'
def parse_top_flatpak(path):
    try:
        import yaml  # type: ignore
        with open(path, encoding="utf-8") as fh:
            return yaml.safe_load(fh)
    except ModuleNotFoundError:
        data = {"finish-args": []}
        in_finish = False
        with open(path, encoding="utf-8") as fh:
            for raw in fh:
                stripped = raw.strip()
                if not stripped or stripped.startswith("#"):
                    continue
                if not raw.startswith(" ") and ":" in stripped:
                    key, _, val = stripped.partition(":")
                    in_finish = key == "finish-args"
                    if not in_finish:
                        data[key] = val.strip().strip("'\"")
                elif in_finish and stripped.startswith("- "):
                    data["finish-args"].append(stripped[2:].strip().strip("'\""))
        return data

manifests = []
for name in ("org.curvebook.CurveBook.yml", "org.curvebook.CurveBook.bundle.yml"):
    manifest = parse_top_flatpak("packaging/linux/flatpak/" + name)
    assert manifest["id"] == "org.curvebook.CurveBook" and manifest["command"] == "curvebook", name
    manifests.append(manifest)
for key in ("runtime", "runtime-version", "sdk", "finish-args"):
    assert manifests[0][key] == manifests[1][key], f"Flatpak manifests disagree on {key}"
print("CurveBook package identity ok")
PYEOF
