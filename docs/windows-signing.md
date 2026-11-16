# Windows signing without Azure

LumenCraft currently builds MSI installers and portable ZIP archives using [`packaging/windows/package.ps1`](../packaging/windows/package.ps1). MSIX packaging is **planned**, not implemented; see [`R8.9`](ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution). The MSIX instructions below apply once a package and its manifest are available.

| Distribution | Signing approach |
|---|---|
| Microsoft Store MSIX | Microsoft signs the package after certification. No Azure Trusted Signing subscription or CA-issued certificate is required for this route. |
| Local MSIX testing | Create a self-signed certificate and explicitly trust its public certificate on each test PC. |
| Local MSI/EXE testing | Self-signing is possible, but it does not create public publisher trust or guarantee removal of Windows security warnings. |
| Public MSI/EXE Store submission | Requires CA-trusted Authenticode signatures for the installer and its PE files; a self-signed certificate cannot satisfy this requirement. Azure is one signing option, not a prerequisite. |

Microsoft documents the [MSIX Store signing rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements#code-signing-for-microsoft-store-submissions) and [MSI/EXE submission requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements). Store signing applies to the Store-distributed package, not to a separately downloaded MSI or EXE.

## Use GitHub when no local Windows PC is available

Run the Windows commands below in a GitHub Actions Windows job. You can edit the workflow, start it, and download its outputs from a browser on macOS. GitHub provides [Windows-hosted runners](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job); the current release workflow already builds Windows MSI/ZIP packages on `windows-latest`.

The dedicated self-signing/MSIX test workflow is **planned in `R8.9`**, not present yet. To prepare that workflow:

1. Reuse the Rust toolchain and WiX setup from the `windows` job in [`.github/workflows/release.yml`](../.github/workflows/release.yml). Start with x64, a bounded job timeout, `workflow_dispatch`, and `permissions: contents: read`; keep this test job separate from release publication.
2. Build with `./packaging/windows/package.ps1 -Arch x64`. Without configured signing credentials this produces unsigned MSI/ZIP files today. MSIX needs the future builder.
3. Run the certificate creation and signing commands below in the Windows job. Use `$env:RUNNER_TEMP` for certificate exports instead of `$env:LOCALAPPDATA`. Keep creation and signing in the same PowerShell step, or pass only the public thumbprint/path between steps and reload the private certificate from `Cert:\CurrentUser\My`.
4. For an MSIX test, explicitly import the public certificate into the disposable runner's `LocalMachine\TrustedPeople` store, verify the package, and attempt installation and CLI smoke tests there. Report runner limitations rather than treating an unavailable interactive GUI as passing human QA.
5. Upload only the intended packages, public `.cer`, thumbprint, checksums, and test logs using an explicit artefact file list. Do not upload the certificate store, private keys, or PFX files. No Azure credentials or signing secrets are needed for a fresh test certificate.
6. Once this manual workflow is added on the default branch, open **GitHub → repository → Actions → the Windows test workflow → Run workflow**. Select the branch, run it, then open the completed run and download its **Artifacts**. See [manually running a workflow](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow) and [downloading artefacts](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts).

A fresh certificate on each run means a new test trust identity. Testers must trust each new public certificate; cross-run update tests needing the same key require a separately managed, persistent signing identity. Generate that identity on Windows CI if needed and retain it in restricted secret storage; never use a downloadable private-key artefact as the handoff. The Store route avoids this test-certificate management for public users.

The existing **Release** workflow is not an isolated self-signing test: non-scheduled runs can create or update a draft release and overwrite its assets. Use its Windows job as a reference; do not run the whole release pipeline just to experiment with certificates.

## 1. Prepare a Windows test machine

Use Windows 10/11 with Windows PowerShell and the Windows SDK Signing Tools installed. Start PowerShell in the repository root. Certificate trust installation below needs an elevated shell on each test PC; package installation runs as the intended Windows user. Organization policy can restrict sideloading.

Ensure `signtool.exe` is on `PATH`, or replace the lookup below with its installed absolute path. The SDK places it under `C:\Program Files (x86)\Windows Kits\10\bin\<SDK-version>\x64\signtool.exe`. See Microsoft's [SignTool reference](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool).

```powershell
$ErrorActionPreference = 'Stop'
$signTool = (Get-Command signtool.exe -ErrorAction Stop).Source
$signingDir = Join-Path $env:LOCALAPPDATA 'LumenCraft\signing'
New-Item -ItemType Directory -Force -Path $signingDir | Out-Null
$cerPath = Join-Path $signingDir 'lumencraft-test.cer'
```

## 2. Create a test certificate

For MSI/EXE tests, use the subject below. For MSIX, first set `$publisher` to the **exact** `Identity Publisher` value from that package's `AppxManifest.xml`, including punctuation and ordering. A friendly display name is not the publisher identity. Obtain the production identity from Partner Center before creating a Store manifest.

```powershell
$publisher = 'CN=LumenCraft Test'
$cert = New-SelfSignedCertificate `
    -Type Custom `
    -Subject $publisher `
    -FriendlyName 'LumenCraft local signing test' `
    -CertStoreLocation 'Cert:\CurrentUser\My' `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -HashAlgorithm SHA256 `
    -KeyUsage DigitalSignature `
    -KeyExportPolicy NonExportable `
    -NotAfter (Get-Date).AddMonths(12) `
    -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')

$thumbprint = $cert.Thumbprint
Export-Certificate -Cert $cert -FilePath $cerPath -NoClobber | Out-Null
$cert | Format-List Subject, Thumbprint, NotAfter
```

Keep this thumbprint for subsequent commands. The private key stays in the signing user's certificate store; the `.cer` contains only the public certificate. Share only that `.cer` with testers, never private keys or PFX files. `-NoClobber` prevents replacing an earlier exported certificate; reuse an existing certificate or choose a new filename for a new test identity.

References: [certificate subject and MSIX requirements](https://learn.microsoft.com/en-us/windows/msix/package/create-certificate-package-signing), [New-SelfSignedCertificate](https://learn.microsoft.com/en-us/powershell/module/pki/new-selfsignedcertificate), and [Export-Certificate](https://learn.microsoft.com/en-us/powershell/module/pki/export-certificate).

To reuse the certificate in a later shell, restore the setup variables from section 1 and set the saved thumbprint explicitly:

```powershell
$thumbprint = 'REPLACE_WITH_YOUR_SAVED_CERTIFICATE_THUMBPRINT'
$cert = Get-Item "Cert:\CurrentUser\My\$thumbprint"
```

## 3. Sign the current MSI/EXE outputs

Build with `packaging/windows/package.ps1` as described in [`releasing.md`](releasing.md). Without signing credentials, its existing `sign.ps1` warns and leaves outputs unsigned. These manual commands use the certificate store directly, so they need neither Azure credentials nor a PFX export.

For example, set `$msiPath` to the MSI that was built, then sign it:

```powershell
$msiPath = (Resolve-Path '.\dist\release\lumencraft-0.1.0-windows-x64.msi').Path
& $signTool sign /fd SHA256 /s My /sha1 $thumbprint $msiPath
if ($LASTEXITCODE -ne 0) { throw 'MSI signing failed' }
```

Adjust the version, architecture, and output directory to your build. The same command signs a standalone `.exe` when given its path. Signing a completed MSI signs the installer container; it does not sign its embedded executables or change copies already inside a portable ZIP. To sign those as well, sign the built EXEs first, repackage with `package.ps1 -SkipBuild`, and then sign the resulting MSI. Package or executable changes after signing invalidate the signature.

`/sha1` selects the certificate by thumbprint; `/fd SHA256` sets the signature digest. This local-test example has no timestamp server dependency; renew and re-sign test packages before the certificate expires. See [SignTool options](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool).

## 4. Sign a future MSIX package

This step needs an actual MSIX produced by the future `R8.9` builder or a separately prepared test package. Today's `package.ps1` does not produce it. Confirm that its manifest publisher matches `$cert.Subject`; if it differs, create a matching certificate before signing.

```powershell
$msixPath = (Resolve-Path '.\dist\release\lumencraft-test-x64.msix').Path
& $signTool sign /fd SHA256 /s My /sha1 $thumbprint $msixPath
if ($LASTEXITCODE -ne 0) { throw 'MSIX signing failed' }
```

The example assumes the MSIX block map uses SHA256, the MakeAppx default. SignTool must use the package's hash algorithm. See [signing an MSIX package](https://learn.microsoft.com/en-us/windows/msix/package/sign-app-package-using-signtool).

## 5. Trust, verify, and install on a test PC

Copy the signed package and public `.cer` to each test PC. Compare the certificate thumbprint with the value obtained from the signing machine through a trusted channel. In an **Administrator PowerShell** on that PC, set the path to the received certificate and inspect it before importing:

```powershell
$cerPath = (Resolve-Path '.\lumencraft-test.cer').Path
$publicCert = Get-PfxCertificate -FilePath $cerPath
$publicCert | Format-List Subject, Thumbprint, NotAfter
```

After verifying that this is the intended test certificate, explicitly trust it:

```powershell
Import-Certificate -FilePath $cerPath `
    -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople' | Out-Null
```

Import into **Trusted People**, not Trusted Root Certification Authorities. Trust affects every user on this PC. See Microsoft's [test-certificate trust guidance](https://learn.microsoft.com/en-us/windows/msix/package/create-certificate-package-signing) and [Import-Certificate reference](https://learn.microsoft.com/en-us/powershell/module/pki/import-certificate).

Back in the intended user's PowerShell, resolve the local package path and verify its signature. For the future MSIX:

```powershell
$signTool = (Get-Command signtool.exe -ErrorAction Stop).Source
$msixPath = (Resolve-Path '.\lumencraft-test-x64.msix').Path
& $signTool verify /pa /v $msixPath
if ($LASTEXITCODE -ne 0) { throw 'Signature verification failed; do not install' }
Add-AppxPackage -Path $msixPath
```

`Add-AppxPackage` installs for the current user; see its [command reference](https://learn.microsoft.com/en-us/powershell/module/appx/add-appxpackage). For an MSI/EXE, inspect the signature with `signtool verify /pa /v` as well, but a self-signed Authenticode certificate can still fail trusted-chain verification. The MSIX Trusted People procedure does not promise ordinary MSI/EXE publisher trust. Do not suppress verification failures or import test certificates into the root store to make a public-release check pass. See [Authenticode trust](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/authenticode). Installing the certificate does not bypass Windows SmartScreen, organizational policy, or Store certification.

## 6. Remove test trust when finished

Uninstall the test app using Windows Settings. In Administrator PowerShell on each test PC, remove only the trusted test certificate with its recorded thumbprint:

```powershell
$thumbprint = 'REPLACE_WITH_YOUR_SAVED_CERTIFICATE_THUMBPRINT'
Remove-Item -LiteralPath "Cert:\LocalMachine\TrustedPeople\$thumbprint"
```

When the signing identity is no longer needed, the signing user can also delete `Cert:\CurrentUser\My\<thumbprint>` from their personal store. Preserve it if further test updates must be signed with the same key. Do not commit certificate files or signing credentials to the repository.

## 7. Publish through Microsoft Store

For the planned MSIX route, create the Store developer account, reserve the product name, and supply the exact Partner Center identity to the package manifest. Test and submit through Partner Center; Microsoft provides the package signature after certification. A self-signed test certificate is not a prerequisite for Store publishing. The implementation and submission checklist is in [`R8.9`](ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution).

These instructions were checked against Microsoft documentation on 2026-11-16. They have not been executed on Windows in this documentation change.
