<#
.SYNOPSIS
  Authenticode-sign files with signtool, using whichever signing material is configured.
#>
param(
  [switch] $RequireSigning,
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
  [string[]] $Files
)
$ErrorActionPreference = 'Stop'

function Write-Warn([string] $Message) {
  if ($env:GITHUB_ACTIONS) { Write-Output "::warning::$Message" } else { Write-Warning $Message }
}

function Find-SignTool {
  if ($env:SIGNTOOL) { return $env:SIGNTOOL }
  $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
  $found = Get-ChildItem -Path $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Sort-Object FullName -Descending | Select-Object -First 1
  if (-not $found) { throw 'signtool.exe not found (install the Windows SDK)' }
  return $found.FullName
}

function Invoke-SignTool([string[]] $Arguments) {
  & $script:SignTool @Arguments
  if ($LASTEXITCODE -ne 0) { throw "signtool $($Arguments[0]) failed with exit code $LASTEXITCODE" }
}

$Files = $Files | ForEach-Object { (Resolve-Path $_).Path }
$haveCert = [bool]$env:WINDOWS_CERTIFICATE
$azureVars = 'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET', 'AZURE_SIGNING_ENDPOINT', 'AZURE_SIGNING_ACCOUNT', 'AZURE_CERT_PROFILE'
$haveAzure = -not ($azureVars | Where-Object { -not [Environment]::GetEnvironmentVariable($_) })

$enforceSigning = $RequireSigning -or ($env:CURVEBOOK_REQUIRE_WINDOWS_SIGNING -match '^(1|true|yes)$')
if (-not $haveCert -and -not $haveAzure) {
  if ($enforceSigning) {
    throw "Windows signing is required (-RequireSigning / CURVEBOOK_REQUIRE_WINDOWS_SIGNING), but neither WINDOWS_CERTIFICATE nor AZURE_* signing secrets are set"
  }
  Write-Warn "Windows signing secrets not set (WINDOWS_CERTIFICATE or AZURE_*): leaving unsigned: $($Files -join ', ')"
  exit 0
}

$script:SignTool = Find-SignTool
$common = @('sign', '/v', '/fd', 'SHA256', '/td', 'SHA256', '/d', 'CurveBook', '/du', 'https://github.com/HauserGroup/curvebook')
$tmp = Join-Path ([IO.Path]::GetTempPath()) "curvebook-sign-$([IO.Path]::GetRandomFileName())"
New-Item -ItemType Directory -Path $tmp | Out-Null

$importedStore = $null
$importedCert = $null
try {
  if ($haveCert) {
    Write-Output "Signing with WINDOWS_CERTIFICATE: $($Files -join ', ')"
    $pfxBytes = [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE)
    $securePwd = $null
    try {
      if ($env:WINDOWS_CERTIFICATE_PASSWORD) {
        $securePwd = ConvertTo-SecureString -String $env:WINDOWS_CERTIFICATE_PASSWORD -AsPlainText -Force
      }
      $flags = [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::UserKeySet -bor
               [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::PersistKeySet
      $importedCert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2
      $importedCert.Import($pfxBytes, $securePwd, $flags)
    }
    finally {
      [Array]::Clear($pfxBytes, 0, $pfxBytes.Length)
      if ($null -ne $securePwd) { $securePwd.Dispose() }
    }
    $importedStore = New-Object System.Security.Cryptography.X509Certificates.X509Store('My', 'CurrentUser')
    $importedStore.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
    $importedStore.Add($importedCert)
    $ts = if ($env:WINDOWS_TIMESTAMP_URL) { $env:WINDOWS_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
    Invoke-SignTool ($common + @('/tr', $ts, '/sha1', $importedCert.Thumbprint) + $Files)
  }
  else {
    Write-Output "Signing with Azure Trusted Signing ($env:AZURE_SIGNING_ACCOUNT / $env:AZURE_CERT_PROFILE): $($Files -join ', ')"
    $trustedSigningVersion = '1.0.95'
    $trustedSigningSha256 = '3bfcf1e0a3cb42af1692f0a8ed45c15de070c2de86f28a59b2795d904d8a920f'
    $pkg = Join-Path $tmp 'trusted-signing.zip'
    $uri = "https://api.nuget.org/v3-flatcontainer/microsoft.trusted.signing.client/$trustedSigningVersion/microsoft.trusted.signing.client.$trustedSigningVersion.nupkg"
    Invoke-WebRequest -Uri $uri -OutFile $pkg
    $actualHash = (Get-FileHash -Path $pkg -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $trustedSigningSha256) {
      throw "Microsoft.Trusted.Signing.Client $trustedSigningVersion checksum mismatch"
    }
    Expand-Archive -Path $pkg -DestinationPath (Join-Path $tmp 'client') -Force
    $dlib = Get-ChildItem -Path (Join-Path $tmp 'client') -Recurse -Filter Azure.CodeSigning.Dlib.dll |
      Where-Object { $_.FullName -match '\\x64\\' } | Select-Object -First 1
    if (-not $dlib) { throw 'Azure.CodeSigning.Dlib.dll not found in Microsoft.Trusted.Signing.Client' }
    $metadata = Join-Path $tmp 'metadata.json'
    @{
      Endpoint               = $env:AZURE_SIGNING_ENDPOINT
      CodeSigningAccountName = $env:AZURE_SIGNING_ACCOUNT
      CertificateProfileName = $env:AZURE_CERT_PROFILE
    } | ConvertTo-Json | Set-Content -Path $metadata -Encoding utf8
    $ts = if ($env:WINDOWS_TIMESTAMP_URL) { $env:WINDOWS_TIMESTAMP_URL } else { 'http://timestamp.acs.microsoft.com' }
    Invoke-SignTool ($common + @('/tr', $ts, '/dlib', $dlib.FullName, '/dmdf', $metadata) + $Files)
  }
  Invoke-SignTool (@('verify', '/pa', '/v') + $Files)
}
finally {
  if ($null -ne $importedStore -and $null -ne $importedCert) {
    try { $importedStore.Remove($importedCert) } catch {}
  }
  if ($null -ne $importedStore) { $importedStore.Dispose() }
  if ($null -ne $importedCert) { $importedCert.Dispose() }
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
