<#
.SYNOPSIS
  Validate a Windows .ico container's header magic, icon type, frame count, and PNG/DIB dimensions.
.DESCRIPTION
  Checks ICONDIR (reserved=0, type=1, count>=1), each ICONDIRENTRY bounds, and verifies every
  embedded frame as either a PNG IHDR chunk or a BITMAPINFOHEADER/BITMAPCOREHEADER DIB matching
  the directory dimensions.
#>
param(
  [string] $Path = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path 'assets\app-icon\curvebook.ico')
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Read-UInt16LE([byte[]] $Bytes, [int] $Offset) {
  return [int]([BitConverter]::ToUInt16($Bytes, $Offset))
}

function Read-UInt32LE([byte[]] $Bytes, [int] $Offset) {
  return [long]([BitConverter]::ToUInt32($Bytes, $Offset))
}

function Read-Int32LE([byte[]] $Bytes, [int] $Offset) {
  return [int]([BitConverter]::ToInt32($Bytes, $Offset))
}

function Read-UInt32BE([byte[]] $Bytes, [int] $Offset) {
  return ([long]$Bytes[$Offset] -shl 24) -bor ([long]$Bytes[$Offset + 1] -shl 16) -bor ([long]$Bytes[$Offset + 2] -shl 8) -bor [long]$Bytes[$Offset + 3]
}

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
  throw "ICO file not found: $Path"
}

$bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path).Path)
if ($bytes.Length -lt 6) {
  throw "Invalid ICO '$Path': file shorter than 6-byte ICONDIR header ($($bytes.Length) bytes)"
}

$reserved = Read-UInt16LE $bytes 0
$iconType = Read-UInt16LE $bytes 2
$count = Read-UInt16LE $bytes 4

if ($reserved -ne 0) {
  throw "Invalid ICO '$Path': ICONDIR reserved field must be 0, got $reserved"
}
if ($iconType -ne 1) {
  throw "Invalid ICO '$Path': ICONDIR type must be 1 (icon), got $iconType"
}
if ($count -lt 1) {
  throw "Invalid ICO '$Path': ICONDIR frame count must be >= 1, got $count"
}

$dirEnd = 6 + (16 * $count)
if ($bytes.Length -lt $dirEnd) {
  throw "Invalid ICO '$Path': truncated ICONDIRENTRY table (need at least $dirEnd bytes, got $($bytes.Length))"
}

$pngSig = [byte[]]@(137, 80, 78, 71, 13, 10, 26, 10)
$summary = New-Object System.Collections.Generic.List[string]

for ($i = 0; $i -lt $count; $i++) {
  $entry = 6 + (16 * $i)
  $rawW = [int]$bytes[$entry]
  $rawH = [int]$bytes[$entry + 1]
  $entryW = if ($rawW -eq 0) { 256 } else { $rawW }
  $entryH = if ($rawH -eq 0) { 256 } else { $rawH }
  $entryReserved = [int]$bytes[$entry + 3]
  if ($entryReserved -ne 0) {
    throw "Invalid ICO '$Path' frame ${i}: ICONDIRENTRY reserved byte must be 0, got $entryReserved"
  }

  $size = Read-UInt32LE $bytes ($entry + 8)
  $offset = Read-UInt32LE $bytes ($entry + 12)
  if ($size -le 0) {
    throw "Invalid ICO '$Path' frame ${i}: dwBytesInRes must be > 0"
  }
  if ($offset -lt $dirEnd -or ($offset + $size) -gt $bytes.Length) {
    throw "Invalid ICO '$Path' frame ${i}: payload range [$offset..$($offset + $size)) outside file bounds ($($bytes.Length))"
  }

  $off = [int]$offset
  $isPng = ($size -ge 24)
  if ($isPng) {
    for ($k = 0; $k -lt 8; $k++) {
      if ($bytes[$off + $k] -ne $pngSig[$k]) {
        $isPng = $false
        break
      }
    }
  }

  if ($isPng) {
    $isIhdr = ($bytes[$off + 12] -eq 73 -and $bytes[$off + 13] -eq 72 -and $bytes[$off + 14] -eq 68 -and $bytes[$off + 15] -eq 82)
    if (-not $isIhdr) {
      throw "Invalid ICO '$Path' frame ${i}: PNG payload missing leading IHDR chunk"
    }
    $pngW = Read-UInt32BE $bytes ($off + 16)
    $pngH = Read-UInt32BE $bytes ($off + 20)
    if ($pngW -lt 1 -or $pngW -gt 256 -or $pngH -lt 1 -or $pngH -gt 256) {
      throw "Invalid ICO '$Path' frame ${i}: PNG dimensions ${pngW}x${pngH} outside 1..=256"
    }
    if ($pngW -ne $entryW -or $pngH -ne $entryH) {
      throw "Invalid ICO '$Path' frame ${i}: directory declares ${entryW}x${entryH}, PNG IHDR has ${pngW}x${pngH}"
    }
    $summary.Add("${entryW}x${entryH}(PNG)")
  }
  else {
    if ($size -lt 16) {
      throw "Invalid ICO '$Path' frame ${i}: DIB payload truncated ($size bytes)"
    }
    $biSize = Read-UInt32LE $bytes $off
    if ($biSize -notin @(12, 40, 52, 56, 108, 124)) {
      throw "Invalid ICO '$Path' frame ${i}: unexpected DIB header size $biSize"
    }
    if ($biSize -eq 12) {
      $dibW = Read-UInt16LE $bytes ($off + 4)
      $dibH = Read-UInt16LE $bytes ($off + 6)
    }
    else {
      $dibW = Read-Int32LE $bytes ($off + 4)
      $dibH = [Math]::Abs((Read-Int32LE $bytes ($off + 8)))
    }
    if ($dibW -ne $entryW -or $dibH -ne (2 * $entryH)) {
      throw "Invalid ICO '$Path' frame ${i}: directory declares ${entryW}x${entryH}, DIB header has ${dibW}x${dibH} (expected ${entryW}x$(2 * $entryH))"
    }
    $summary.Add("${entryW}x${entryH}(DIB)")
  }
}

Write-Output ("PASS {0}: {1} frame(s) [{2}]" -f $Path, $count, ($summary -join ', '))
