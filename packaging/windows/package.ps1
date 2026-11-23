<#
.SYNOPSIS
  Build, sign and package CurveBook for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    curvebook-<version>-windows-<arch>.msi            installer (WiX v5)
#>
param(
  [ValidateSet('x64', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild,
  [switch] $RequireSigning
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

$Version = $env:CURVEBOOK_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
$MsiVersion = ($Version -split '[-+]')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

if (-not $env:CURVEBOOK_BUILD_SHA) { $env:CURVEBOOK_BUILD_SHA = (git -C $Root rev-parse HEAD 2>$null) }
if (-not $env:CURVEBOOK_BUILD_DATE) { $env:CURVEBOOK_BUILD_DATE = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd') }

Write-Output "CurveBook $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  $env:CURVEBOOK_REQUIRE_WINRES = '1'
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p curvebook -p curvebook-cli --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\release"

function Get-PeHeader([string] $Path) {
  $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
  try {
    $len = $stream.Length
    if ($len -lt 0x40) { throw "$Path is too small to be a valid PE binary ($len bytes)" }
    $dos = New-Object byte[] 0x40
    if ($stream.Read($dos, 0, 0x40) -ne 0x40) { throw "Failed to read DOS header from $Path" }
    if ($dos[0] -ne 0x4D -or $dos[1] -ne 0x5A) { throw "$Path lacks MZ DOS signature" }
    $pe = [BitConverter]::ToInt32($dos, 0x3C)
    if ($pe -lt 0x40 -or ($pe + 0x5E) -gt $len) { throw "$Path has invalid PE offset $pe (file length $len)" }
    $null = $stream.Seek([long]$pe, [System.IO.SeekOrigin]::Begin)
    $hdr = New-Object byte[] 0x5E
    if ($stream.Read($hdr, 0, 0x5E) -ne 0x5E) { throw "Failed to read PE header from $Path" }
    if ($hdr[0] -ne 0x50 -or $hdr[1] -ne 0x45 -or $hdr[2] -ne 0x00 -or $hdr[3] -ne 0x00) {
      throw "$Path lacks PE\0\0 signature at offset $pe"
    }
    return @{ Machine = [BitConverter]::ToUInt16($hdr, 4); Subsystem = [BitConverter]::ToUInt16($hdr, 0x5C) }
  }
  finally {
    $stream.Dispose()
  }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'arm64' { 0xAA64 } }
foreach ($check in @(@('curvebook.exe', 2), @('curvebook-cli.exe', 3))) {
  $h = Get-PeHeader (Join-Path $Bin $check[0])
  if ($h.Machine -ne $Machine) { throw "$($check[0]) is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
  if ($h.Subsystem -ne $check[1]) { throw "$($check[0]) has PE subsystem $($h.Subsystem), expected $($check[1])" }
  Write-Output "ok $($check[0]): $Arch, PE subsystem $($h.Subsystem)"
}
$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin 'curvebook.exe'), (Join-Path $Bin 'curvebook-cli.exe') $Stage

$signFlags = @{}
if ($RequireSigning -or ($env:CURVEBOOK_REQUIRE_WINDOWS_SIGNING -match '^(1|true|yes)$')) {
  $signFlags['RequireSigning'] = $true
}
& (Join-Path $PSScriptRoot 'sign.ps1') @signFlags (Join-Path $Stage 'curvebook.exe') (Join-Path $Stage 'curvebook-cli.exe')

$Docs = Join-Path $Stage 'docs'
New-Item -ItemType Directory -Force -Path $Docs | Out-Null
foreach ($f in 'README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'COPYRIGHT', 'NOTICE', 'ATTRIBUTION.md', 'ASSETS.md') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Docs }
}
if ($env:CURVEBOOK_FONTS_DIR) {
  foreach ($ofl in Get-ChildItem -Path (Join-Path $env:CURVEBOOK_FONTS_DIR 'fonts\*\OFL.txt') -ErrorAction SilentlyContinue) {
    Copy-Item $ofl.FullName (Join-Path $Docs "OFL-$($ofl.Directory.Name).txt")
  }
}
if (Test-Path (Join-Path $Root 'licenses')) {
  Copy-Item -Recurse (Join-Path $Root 'licenses') $Docs
}

function New-InstallerArt([string] $IconPng, [string] $BannerOut, [string] $DialogOut) {
  Add-Type -AssemblyName System.Drawing
  $paper = [System.Drawing.ColorTranslator]::FromHtml('#eef4fb')
  $accent = [System.Drawing.ColorTranslator]::FromHtml('#1b2d56')
  $icon = [System.Drawing.Image]::FromFile($IconPng)
  try {
    foreach ($spec in @(@($BannerOut, 493, 58), @($DialogOut, 493, 312))) {
      $bmp = New-Object System.Drawing.Bitmap $spec[1], $spec[2], ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      try {
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.Clear([System.Drawing.Color]::White)
        if ($spec[2] -eq 58) {
          $g.DrawImage($icon, 493 - 46 - 8, 6, 46, 46)
        } else {
          $panel = New-Object System.Drawing.SolidBrush $paper
          $stripe = New-Object System.Drawing.SolidBrush $accent
          $g.FillRectangle($panel, 0, 0, 164, 312)
          $g.FillRectangle($stripe, 160, 0, 4, 312)
          $g.DrawImage($icon, 18, 64, 124, 124)
          $panel.Dispose(); $stripe.Dispose()
        }
      } finally { $g.Dispose() }
      $bmp.Save($spec[0], [System.Drawing.Imaging.ImageFormat]::Bmp)
      $bmp.Dispose()
    }
  } finally { $icon.Dispose() }
}
$Banner = Join-Path $Stage 'installer-banner.bmp'
$Dialog = Join-Path $Stage 'installer-dialog.bmp'
New-InstallerArt (Join-Path $Root 'assets\app-icon\hicolor\256x256\apps\org.curvebook.CurveBook.png') $Banner $Dialog

$Msi = Join-Path $Dist "curvebook-$Version-windows-$Arch.msi"
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'curvebook.wxs') -arch $Arch `
    -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -culture en-US -loc (Join-Path $PSScriptRoot 'curvebook.en-us.wxl') `
    -d "Version=$MsiVersion" -d "DisplayVersion=$Version" -d "BinDir=$Stage" -d "IconPath=$(Join-Path $Root 'assets\app-icon\curvebook.ico')" `
    -d "DocsDir=$Docs" -d "BannerBmp=$Banner" -d "DialogBmp=$Dialog" `
    -o $Msi
}
& (Join-Path $PSScriptRoot 'check-install-policy.ps1') -MsiPath $Msi
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') @signFlags $Msi

$HostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($Arch -ne 'arm64' -or $HostArch -eq 'arm64') {
  Invoke-Native 'curvebook-cli --version' { & (Join-Path $Stage 'curvebook-cli.exe') --version }
} else {
  Write-Output "skipping curvebook-cli --version: an $Arch build doesn't run on this $HostArch machine"
}
Get-Item $Msi | Format-Table Name, Length
