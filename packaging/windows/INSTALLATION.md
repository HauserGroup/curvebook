# Windows installation policy checks

New installations default to the current account under `Programs\CurveBook`.
The first setup page offers installation for everyone on the machine. Scope is selected before
duplicate checks and directory costing. An update preserves the existing context's remembered
folder; an explicit `INSTALLFOLDER` is never overwritten and survives elevation as a secure MSI
property. Silent machine updates pass `ALLUSERS=1`. Cross-context duplicate installations are
refused until the opposite-scope installation is removed.

CurveBook uses its own fresh `UpgradeCode`, explicit component GUIDs, installation directories,
and `Software\CurveBook` / `App Paths\curvebook.exe` registry keys so it installs alongside any
existing LumenCraft installation without uninstalling, upgrading, or modifying LumenCraft.

## Repeatable metadata tests

On Windows with WiX 5.0.2 and its UI/Util extensions:

```powershell
./packaging/windows/test-install-policy.ps1 -OutputDirectory ./target/install-policy-tests
```

The script compiles x64 and arm64 metadata-only MSI fixtures with non-executable placeholder
payloads and verifies scope guards, folder preservation, action sequencing, upgrade/component
identity, and the registry contract in `install-policy.json` using a read-only Windows Installer
session.

For user-facing Windows `.msi` installation (`x64` and `arm64`) and verification steps, see [Get Started](https://curvebook.org/manual/latest/get-started.html) (`docs/reference/get-started.md`).
