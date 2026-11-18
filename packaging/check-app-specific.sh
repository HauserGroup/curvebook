#!/usr/bin/env bash
# App-specific package assertions retained alongside shared packaging lint.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

python3 - <<'PYEOF'
import json
import xml.etree.ElementTree as ET

APP_ID = "org.curvebook.CurveBook"
FORBIDDEN = tuple(
    "".join(parts)
    for parts in (
        ("art", "craft"),
        ("get", "art", "craft"),
        ("story", "told"),
        ("story", "teller"),
        ("vector", "craft"),
        ("draw", "craft"),
        ("p", "craft"),
        ("pixel", "craft"),
        ("lumen", "craft"),
    )
)

def check_clean(label, text):
    lower = text.lower()
    for token in FORBIDDEN:
        assert token not in lower, f"{label} contains forbidden leftover `{token}`"

with open(f"packaging/linux/{APP_ID}.desktop", encoding="utf-8") as fh:
    desktop = fh.read()
check_clean("desktop", desktop)
assert f"Icon={APP_ID}" in desktop
assert "Exec=curvebook %F" in desktop
assert "StartupWMClass=curvebook" in desktop

with open(f"packaging/linux/{APP_ID}.metainfo.xml.in", encoding="utf-8") as fh:
    metainfo_raw = fh.read()
check_clean("metainfo", metainfo_raw)
metainfo = ET.fromstring(metainfo_raw)
assert metainfo.findtext("id") == APP_ID
assert metainfo.findtext("name") == "CurveBook"
assert metainfo.findtext("launchable") == f"{APP_ID}.desktop"
assert metainfo.findtext("provides/binary") == "curvebook"

with open(f"packaging/linux/{APP_ID}.mime.xml", encoding="utf-8") as fh:
    mime = fh.read()
check_clean("mime", mime)
assert "*.curvebook" in mime
assert "*.cbtemplate" in mime

with open("packaging/macos/Info.plist.in", encoding="utf-8") as fh:
    plist = fh.read()
check_clean("Info.plist.in", plist)
assert APP_ID in plist
assert "curvebook" in plist

with open("packaging/windows/install-policy.json", encoding="utf-8") as fh:
    win_policy = json.load(fh)
check_clean("install-policy.json", json.dumps(win_policy))
assert win_policy["id"] == "curvebook"
assert win_policy["name"] == "CurveBook"
assert win_policy["upgradeCode"] == "570875D8-A8A5-4335-8646-C8CE385B27BD"
assert win_policy["componentGuids"]["CurveBookApp"] == "30074268-38AD-4435-82B7-E2E8948273F9"
assert win_policy["componentGuids"]["DesktopShortcut"] == "5BFAC2A4-683C-40C9-B561-298D76308811"

print("CurveBook package identity ok")
PYEOF
