# Windows signing without Azure

CurveBook builds Windows `x64` and `arm64` MSI installers (`x86` and portable ZIP archives were retired in [`R0.28`](../history/roadmap/r0.md#r028-rein-in-the-number-of-release-install-files)) using [`packaging/windows/package.ps1`](../../packaging/windows/package.ps1). Beta `.msi` installers ship **unsigned** ([`R8.10`](../history/roadmap/r8.md#r810-signed-macos-installers--download-verification-beta-gate); Windows Authenticode signing moves to [`R8.11`](../ROADMAP.md#r811-windows-authenticode-signing-after-beta)), covered by `SHA256SUMS.txt` and Sigstore attestations ([verifying downloads](verifying-downloads.md)). Users install unsigned Beta `.msi` files via SmartScreen **More info → Run anyway**; Windows 11 Smart App Control or managed PCs may block unsigned installers until `R8.11` or Microsoft Store MSIX (`R8.9`). The separate [`Windows Store package`](../../.github/workflows/windows-store.yml) workflow builds an unsigned x64 MSIX for Store submission. It does not use Azure or signing secrets. Windows installation, CLI/MCP alias behavior, upgrade/uninstall, and certification evidence are still required under [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution).

| Distribution | Signing approach |
|---|---|
| Microsoft Store MSIX | Microsoft signs the package after certification. No Azure Trusted Signing subscription or CA-issued certificate is required for this route. |
| Local MSIX testing | Create a self-signed certificate and explicitly trust its public certificate on each test PC. |
| Local MSI/EXE testing | Self-signing is possible, but it does not create public publisher trust or guarantee removal of Windows security warnings. |
| Public MSI/EXE Store submission | Requires CA-trusted Authenticode signatures for the installer and its PE files; a self-signed certificate cannot satisfy this requirement. Azure is one signing option, not a prerequisite. |

Microsoft documents the [MSIX Store signing rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements#code-signing-for-microsoft-store-submissions) and [MSI/EXE submission requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements). Store signing applies to the Store-distributed package, not to a separately downloaded MSI or EXE.

## Use GitHub when no local Windows PC is available

Use the **Windows Store package** workflow from a browser on macOS:

1. Open the repository's **Actions → Windows Store package → Run workflow** once this workflow has landed on the default branch.
2. Choose the branch to build. For the first Store submission, use package version **`1.0.0.0`**.
3. Start the workflow and wait for both jobs to succeed.
4. Open the completed run and download **`curvebook-store-x64`** from **Artifacts**. Extract the downloaded ZIP.
5. In Partner Center, open **CurveBook → Start your submission → Packages** and upload the extracted **`.msix`** file. The XML manifest and `SHA256SUMS.txt` are included for review and integrity checking; they are not separate Store uploads.

A pull request changing the Store builder also runs this scoped workflow. It generates third-party licence notices, builds the desktop/CLI with a static C runtime, validates the payload's PE architectures, runs package tests and a CLI version smoke check, and packages with Windows SDK `MakeAppx.exe` validation enabled. It uploads Actions artifacts, creates no GitHub Release, and does not submit or publish to Microsoft Store.

**Store identity:** the maintainer supplied the reserved name and publisher on 2026-11-18. Public package identifiers live in [`store-identity.json`](../../packaging/windows/store-identity.json): `JakobSture.CurveBook`, `CN=8373FC7D-82BB-4F48-BE69-7A5260B5630A`, and publisher display name `Jakob Sture`. These are not signing credentials.

**Versioning:** the Store package version is separate from the Cargo/Alpha version. It must have four integer components, a first component of at least `1`, and a final component of `0`. After a version is accepted by Store, increase the version for an update (for example `1.0.1.0`). Never automatically reset it when Cargo's version changes. The package filename retains the Cargo version so the source release remains identifiable. See [Microsoft's package version rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements#package-version-numbering).

**This unsigned MSIX is for Store submission.** It is not a public sideload installer. Microsoft signs the package after certification. No Azure account, CA certificate, or GitHub signing secret is needed. The test self-signing/install workflow remains pending; the manual PowerShell procedure below is for explicitly trusted test machines.

The main **Release** workflow builds unsigned `x64` and `arm64` MSI installers for Beta (and guards signing secrets against nightly scheduled runs per `R8.11`); Store signing does not sign those downloads. Do not run the full release matrix to experiment with MSIX packaging. Store listing text, screenshots, privacy disclosures, age rating, certification, and installed-app testing still need completion before publishing.

## 1. Prepare a Windows test machine

Use Windows 10/11 with Windows PowerShell and the Windows SDK Signing Tools installed. Start PowerShell in the repository root. Certificate trust installation below needs an elevated shell on each test PC; package installation runs as the intended Windows user. Organization policy can restrict sideloading.

Ensure `signtool.exe` is on `PATH`, or replace the lookup below with its installed absolute path. The SDK places it under `C:\Program Files (x86)\Windows Kits\10\bin\<SDK-version>\x64\signtool.exe`. See Microsoft's [SignTool reference](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool).

```powershell
$ErrorActionPreference = 'Stop'
$signTool = (Get-Command signtool.exe -ErrorAction Stop).Source
$signingDir = Join-Path $env:LOCALAPPDATA 'CurveBook\signing'
New-Item -ItemType Directory -Force -Path $signingDir | Out-Null
$cerPath = Join-Path $signingDir 'curvebook-test.cer'
```

## 2. Create a test certificate

For MSI/EXE tests, use the subject below. For MSIX, first set `$publisher` to the **exact** `Identity Publisher` value from that package's `AppxManifest.xml`, including punctuation and ordering. A friendly display name is not the publisher identity. Obtain the production identity from Partner Center before creating a Store manifest.

```powershell
$publisher = 'CN=CurveBook Test'
$cert = New-SelfSignedCertificate `
    -Type Custom `
    -Subject $publisher `
    -FriendlyName 'CurveBook local signing test' `
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
$msiPath = (Resolve-Path '.\dist\release\curvebook-0.1.0-windows-x64.msi').Path
& $signTool sign /fd SHA256 /s My /sha1 $thumbprint $msiPath
if ($LASTEXITCODE -ne 0) { throw 'MSI signing failed' }
```

Adjust the version, architecture, and output directory to your build. The same command signs a standalone `.exe` when given its path. Signing a completed MSI signs the installer container; it does not sign its embedded executables. To sign those as well, sign the built EXEs first, repackage with `package.ps1 -SkipBuild`, and then sign the resulting MSI. Package or executable changes after signing invalidate the signature.

`/sha1` selects the certificate by thumbprint; `/fd SHA256` sets the signature digest. This local-test example has no timestamp server dependency; renew and re-sign test packages before the certificate expires. See [SignTool options](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool).

## 4. Sign a future MSIX package

This step needs an actual MSIX produced by the future `R8.9` builder or a separately prepared test package. Today's `package.ps1` does not produce it. Confirm that its manifest publisher matches `$cert.Subject`; if it differs, create a matching certificate before signing.

```powershell
$msixPath = (Resolve-Path '.\dist\release\curvebook-test-x64.msix').Path
& $signTool sign /fd SHA256 /s My /sha1 $thumbprint $msixPath
if ($LASTEXITCODE -ne 0) { throw 'MSIX signing failed' }
```

The example assumes the MSIX block map uses SHA256, the MakeAppx default. SignTool must use the package's hash algorithm. See [signing an MSIX package](https://learn.microsoft.com/en-us/windows/msix/package/sign-app-package-using-signtool).

## 5. Trust, verify, and install on a test PC

Copy the signed package and public `.cer` to each test PC. Compare the certificate thumbprint with the value obtained from the signing machine through a trusted channel. In an **Administrator PowerShell** on that PC, set the path to the received certificate and inspect it before importing:

```powershell
$cerPath = (Resolve-Path '.\curvebook-test.cer').Path
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
$msixPath = (Resolve-Path '.\dist\release\curvebook-test-x64.msix').Path
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

The Store developer account, reserved product name, and Partner Center identity are configured. Build the package with the workflow above. Test and submit through Partner Center; Microsoft provides the package signature after certification. A self-signed test certificate is not a prerequisite for Store publishing. The implementation and submission checklist is in [`R8.9`](../ROADMAP.md#r89-windows-msix-packaging--microsoft-store-distribution).

Store identity and workflow instructions updated on 2026-11-18. Initial x64 package construction, CLI version smoke check, and Windows SDK validation passed in [GitHub run `44301984148`](https://github.com/HauserGroup/curvebook/actions/runs/44301984148); the downloaded artifact's checksums and contents were also checked. The manual self-signing commands and installed MSIX behavior remain unverified.
