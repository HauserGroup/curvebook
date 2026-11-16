<#
.SYNOPSIS
  Build non-executable installer metadata fixtures; never install them.
#>
param(
  [Parameter(Mandatory = $true)] [string] $OutputDirectory,
  [ValidateSet('x64','x86','arm64')] [string[]] $Architectures = @('x64','x86','arm64')
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$policy = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'install-policy.json') | ConvertFrom-Json
if ($policy.upgradeCode -ne '570875D8-A8A5-4335-8646-C8CE385B27BD') {
  throw "Unexpected UpgradeCode '$($policy.upgradeCode)' in install-policy.json."
}
if ($policy.componentGuids.CurveBookApp -ne '30074268-38AD-4435-82B7-E2E8948273F9' -or
    $policy.componentGuids.DesktopShortcut -ne '5BFAC2A4-683C-40C9-B561-298D76308811') {
  throw 'Unexpected component GUIDs in install-policy.json.'
}
[void](New-Item -ItemType Directory -Force -Path $OutputDirectory)
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
Add-Type -AssemblyName System.Drawing
foreach ($arch in $Architectures) {
  $stage = Join-Path $output $arch
  [void](New-Item -ItemType Directory -Force -Path $stage)
  foreach ($name in $policy.payloads) {
    $destination = Join-Path $stage $name
    [void](New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($destination)))
    Set-Content -LiteralPath $destination -Encoding Ascii -Value 'NON-EXECUTABLE MSI METADATA TEST FIXTURE'
  }
  foreach ($folder in @('docs','notices')) {
    $path = Join-Path $stage $folder
    [void](New-Item -ItemType Directory -Force -Path $path)
    Set-Content -LiteralPath (Join-Path $path "$folder-fixture.txt") -Value 'Metadata test fixture.'
  }
  foreach ($spec in @(@('banner.bmp',493,58),@('dialog.bmp',493,312))) {
    $bitmap = [Drawing.Bitmap]::new($spec[1],$spec[2],[Drawing.Imaging.PixelFormat]::Format24bppRgb)
    try { $bitmap.Save((Join-Path $stage $spec[0]),[Drawing.Imaging.ImageFormat]::Bmp) }
    finally { $bitmap.Dispose() }
  }
  $sources = @($policy.sources | ForEach-Object { Join-Path $PSScriptRoot $_ })
  $msi = Join-Path $stage "$($policy.id)-NOT-INSTALLABLE.msi"
  $extensions = @($policy.extensions | ForEach-Object { '-ext'; "WixToolset.$_.wixext" })
  & wix build @sources -arch $arch @extensions `
    -culture en-US -d 'Version=0.0.1' -d 'DisplayVersion=0.0.1-metadata-test' `
    -d "BinDir=$stage" -d "IconPath=$(Join-Path $root $policy.icon)" `
    -d "DocsDir=$(Join-Path $stage 'docs')" `
    -d "BannerBmp=$(Join-Path $stage 'banner.bmp')" -d "DialogBmp=$(Join-Path $stage 'dialog.bmp')" `
    -o $msi @($policy.localization | ForEach-Object { '-loc'; Join-Path $PSScriptRoot $_ })
  if ($LASTEXITCODE -ne 0) { throw "WiX validation failed for $arch (exit $LASTEXITCODE)." }
  $shell = (Get-Process -Id $PID).Path
  & $shell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'check-install-policy.ps1') -MsiPath $msi
  if ($LASTEXITCODE -ne 0) { throw "Compiled installer policy failed for $arch (exit $LASTEXITCODE)." }
  Write-Output "PASS $arch metadata-only package. This is not an executable installer."
}
