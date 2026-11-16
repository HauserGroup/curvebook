<#
.SYNOPSIS
  Check the compiled installation policy without installing any product.
.DESCRIPTION
  Uses Windows Installer's safe session (OpenPackage option 1). Only property-setting custom
  actions are exercised. No AppSearch, install, repair, removal or file-writing action runs.
#>
param([Parameter(Mandatory = $true)] [string] $MsiPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$policy = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'install-policy.json') | ConvertFrom-Json
function Read-Rows($Database, [string] $Sql, [int] $Columns) {
  $view = $Database.OpenView($Sql)
  try {
    [void]$view.Execute()
    while ($null -ne ($record = $view.Fetch())) {
      try {
        $values = @(for ($i = 1; $i -le $Columns; $i++) {
          $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, @($i))
        })
        ,$values
      } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) }
    }
  } finally {
    [void]$view.Close()
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
  }
}
function Set-SessionProperty($Session, [string] $Name, [string] $Value) {
  [void]$Session.GetType().InvokeMember('Property', 'SetProperty', $null, $Session, @($Name, $Value))
}
function Get-SessionProperty($Session, [string] $Name) {
  $Session.GetType().InvokeMember('Property', 'GetProperty', $null, $Session, @($Name))
}
function Get-PackageTemplate($Database) {
  $summary = $Database.GetType().InvokeMember('SummaryInformation', 'GetProperty', $null, $Database, @(0))
  try { [string]$summary.GetType().InvokeMember('Property', 'GetProperty', $null, $summary, @(7)) }
  finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($summary) }
}
$installer = $null; $database = $null; $session = $null
try {
  $installer = New-Object -ComObject WindowsInstaller.Installer
  $file = (Resolve-Path -LiteralPath $MsiPath).Path
  $database = $installer.OpenDatabase($file, 0)
  $session = $installer.OpenPackage($file, 1)
  $properties = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Property`, `Value` FROM `Property`' 2)) { $properties[$row[0]] = $row[1] }
  $secureProperties = @($properties['SecureCustomProperties'] -split ';')
  if ('INSTALLFOLDER' -notin $secureProperties) {
    throw 'The selected installation folder must survive elevation (secure INSTALLFOLDER).'
  }
  if ($properties['ALLUSERS'] -ne '2' -or $properties['MSIINSTALLPERUSER'] -ne '1') {
    throw 'Policy regression: new packages must default to the per-user dual-scope context.'
  }
  if ($properties['UpgradeCode'].Trim('{}') -ne $policy.upgradeCode) { throw 'The existing upgrade identity must not change.' }
  if ($properties['UpgradeCode'].Trim('{}') -eq 'CC9EF573-33E0-4208-BE7A-690B34A5A4C9') {
    throw 'CurveBook must use a fresh UpgradeCode separate from LumenCraft.'
  }
  $components = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Component`, `ComponentId` FROM `Component`' 2)) {
    $components[$row[0]] = $row[1].Trim('{}')
  }
  foreach ($entry in $policy.componentGuids.PSObject.Properties) {
    if (-not $components.ContainsKey($entry.Name) -or $components[$entry.Name] -ne $entry.Value) {
      throw "Component '$($entry.Name)' GUID must match pinned policy value '$($entry.Value)'."
    }
    if ($components[$entry.Name] -in @('039825BB-45BE-41D1-BBAF-8D25A4125026', '512791E8-A25F-40EC-82B4-500A4EE18F8F')) {
      throw "Component '$($entry.Name)' must use a fresh GUID separate from LumenCraft."
    }
  }
  $directories = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Directory`, `Directory_Parent`, `DefaultDir` FROM `Directory`' 3)) {
    $directories[$row[0]] = @($row[1], $row[2])
  }
  $installLongName = ($directories['INSTALLFOLDER'][1] -split '\|')[-1]
  if ($installLongName -ne $policy.name) {
    throw "Expected INSTALLFOLDER directory name '$($policy.name)', got '$($directories['INSTALLFOLDER'][1])'."
  }
  $programs = $directories['INSTALLFOLDER'][0]
  if ($programs -eq 'ProgramFiles6432Folder') { $programs = $directories[$programs][0] }
  if ($programs -notin 'ProgramFilesFolder', 'ProgramFiles64Folder') {
    throw 'Use Windows Installer native folder redirection directly under ProgramFiles, not a hard-coded profile path.'
  }
  $guards = @(Read-Rows $database 'SELECT `Condition`, `Description` FROM `LaunchCondition`' 2 |
    Where-Object { $_[0] -match 'CURVEBOOK_(USER|MACHINE)_DIR' })
  if ($guards.Count -ne 2) { throw 'Expected both duplicate-installation guards.' }
  $cases = 0
  foreach ($scope in @('', '2', '1')) {
    foreach ($user in @('', 'D:\Existing user app')) {
      foreach ($machine in @('', 'E:\Existing machine app')) {
        foreach ($installed in @('', '1')) {
          Set-SessionProperty $session 'ALLUSERS' $scope
          Set-SessionProperty $session 'Installed' $installed
          Set-SessionProperty $session 'CURVEBOOK_USER_DIR' $user
          Set-SessionProperty $session 'CURVEBOOK_MACHINE_DIR' $machine
          $allowed = $true
          foreach ($guard in $guards) {
            $value = $session.EvaluateCondition($guard[0])
            if ($value -notin 0, 1) { throw 'Invalid duplicate guard.' }
            if ($value -eq 0) { $allowed = $false }
          }
          $opposite = if ($scope -eq '1') { $user } else { $machine }
          $expected = [bool]$installed -or -not [bool]$opposite
          if ($allowed -ne $expected) { throw "Scope regression: ALLUSERS='$scope', user=$([bool]$user), machine=$([bool]$machine), maintenance=$([bool]$installed)." }
          $cases++
        }
      }
    }
  }
  foreach ($scope in @('', '2', '1')) {
    foreach ($explicit in @('', 'F:\Chosen exact app')) {
      Set-SessionProperty $session 'ALLUSERS' $scope
      Set-SessionProperty $session 'INSTALLFOLDER' $explicit
      Set-SessionProperty $session 'CURVEBOOK_USER_DIR' 'D:\Existing user app'
      Set-SessionProperty $session 'CURVEBOOK_MACHINE_DIR' 'E:\Existing machine app'
      $action = if ($scope -eq '1') { 'CurveBookRestoreMachineFolder' } else { 'CurveBookRestoreUserFolder' }
      $restore = @(Read-Rows $database ('SELECT `Condition` FROM `InstallExecuteSequence` WHERE `Action` = ''{0}''' -f $action) 1)[0]
      $selected = $session.EvaluateCondition($restore[0]) -eq 1
      if ($selected -eq [bool]$explicit) { throw "$action must run only when no folder was supplied." }
      if ($selected -and $session.DoAction($action) -ne 1) { throw 'Remembered-folder property action failed.' }
      $expected = if ($explicit) { $explicit } elseif ($scope -eq '1') { 'E:\Existing machine app' } else { 'D:\Existing user app' }
      if ((Get-SessionProperty $session 'INSTALLFOLDER') -ne $expected) { throw 'Custom or remembered folder was not preserved.' }
    }
  }
  $ui = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Action`, `Sequence` FROM `InstallUISequence`' 2)) { $ui[$row[0]] = [int]$row[1] }
  if (-not ($ui['AppSearch'] -lt $ui['CurveBookPreferMachineScope'] -and
      $ui['CurveBookPreferMachineScope'] -lt $ui['CurveBookScopeDlg'] -and
      $ui['CurveBookScopeDlg'] -lt $ui['FindRelatedProducts'] -and
      $ui['FindRelatedProducts'] -lt $ui['LaunchConditions'] -and
      $ui['CurveBookScopeDlg'] -lt $ui['CurveBookRestoreUserFolder'] -and $ui['CurveBookRestoreUserFolder'] -lt $ui['CostInitialize'] -and
      $ui['CurveBookScopeDlg'] -lt $ui['CurveBookRestoreMachineFolder'] -and $ui['CurveBookRestoreMachineFolder'] -lt $ui['CostInitialize'])) {
    throw 'Scope must be chosen after AppSearch and before FindRelatedProducts, launch checks and costing.'
  }
  $defaults = @(Read-Rows $database 'SELECT `Action`, `Sequence`, `Condition` FROM `InstallUISequence`' 3 | Where-Object { $_[0] -like 'CurveBookDefault*' })
  if ($defaults.Count -lt 2) { throw 'The wizard must set the new-install folder after the scope page.' }
  foreach ($row in $defaults) {
    if (-not ($ui['CurveBookScopeDlg'] -lt [int]$row[1] -and [int]$row[1] -lt $ui['CostInitialize'])) { throw "$($row[0]) must run after the scope page and before costing." }
  }
  if (@(Read-Rows $database 'SELECT `Action` FROM `InstallExecuteSequence`' 1 | Where-Object { $_[0] -like 'CurveBookDefault*' }).Count) {
    throw 'Silent installs must not use the wizard-only default folder actions.'
  }
  $machineRoot = if ((Get-PackageTemplate $database) -match '^Intel') {
    if (Get-SessionProperty $session 'VersionNT64') { ${env:ProgramFiles(x86)} } else { $env:ProgramFiles }
  } elseif ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
  $machineFolder = (Join-Path $machineRoot $policy.name) + '\'
  foreach ($scope in @('1', '')) {
    Set-SessionProperty $session 'ALLUSERS' $scope
    Set-SessionProperty $session 'INSTALLFOLDER' ''
    Set-SessionProperty $session 'CURVEBOOK_MACHINE_DIR' ''
    Set-SessionProperty $session 'CURVEBOOK_USER_DIR' ''
    foreach ($row in @($defaults | Sort-Object { [int]$_[1] })) {
      if ($session.EvaluateCondition($row[2]) -eq 1 -and $session.DoAction($row[0]) -ne 1) { throw "$($row[0]) failed." }
    }
    $folder = Get-SessionProperty $session 'INSTALLFOLDER'
    $wanted = if ($scope -eq '1') { $machineFolder } else { (Join-Path $env:LOCALAPPDATA "Programs\$($policy.name)") + '\' }
    if ($folder -ine $wanted) { throw "Wizard default folder for ALLUSERS='$scope' is '$folder', expected '$wanted'." }
  }
  $registry = @(Read-Rows $database 'SELECT `Root`, `Key`, `Name`, `Value` FROM `Registry`' 4)
  if (@($registry | Where-Object { $_[0] -in '1', '2' }).Count) { throw 'Registration must follow the selected context, not force HKCU or HKLM.' }
  foreach ($entry in $policy.registry) {
    $found = @($registry | Where-Object { $_[0] -eq $entry[0] -and $_[1] -eq $entry[1] -and $_[2] -eq $entry[2] -and $_[3] -eq $entry[3] })
    if ($found.Count -ne 1) { throw "Lost or changed registration: $($entry[1]) / $($entry[2])" }
  }
  Write-Output "PASS $($policy.name): $cases scope cases, folder preservation, action sequencing and registry contract. No installation performed."
} finally {
  foreach ($com in @($session, $database, $installer)) {
    if ($null -ne $com) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($com) }
  }
}
