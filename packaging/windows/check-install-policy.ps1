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
  # Summary information property 7 is the platform: "Intel;1033", "x64;1033" or "Arm64;1033".
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
  if ($policy.mode -eq 'perUser') {
    if ($properties.ContainsKey('ALLUSERS')) { throw 'Launcher must remain per-user; machine self-update is not part of this change.' }
    if ($properties['UpgradeCode'].Trim('{}') -ne $policy.upgradeCode) { throw 'Launcher upgrade identity changed.' }
    $dirs = @{}
    foreach ($row in (Read-Rows $database 'SELECT `Directory`, `Directory_Parent`, `DefaultDir` FROM `Directory`' 3)) { $dirs[$row[0]] = @($row[1],$row[2]) }
    if ($dirs['INSTALLFOLDER'][0] -ne 'CraftSuiteFolder' -or $dirs['CraftSuiteFolder'][1] -ne 'ArtCraft' -or
        $dirs['CraftSuiteFolder'][0] -ne 'LocalProgramsFolder' -or $dirs['LocalProgramsFolder'][0] -ne 'LocalAppDataFolder') {
      throw 'Launcher new-install folder must be under the per-user ArtCraft suite root.'
    }
    foreach ($explicit in @('', 'F:\Chosen exact launcher')) {
      Set-SessionProperty $session 'PREVIOUSINSTALLFOLDER' 'E:\Existing exact launcher'
      Set-SessionProperty $session 'INSTALLFOLDER' $explicit
      $row = @(Read-Rows $database 'SELECT `Condition` FROM `InstallUISequence` WHERE `Action` = ''CraftRestoreLauncherFolder''' 1)[0]
      if ($session.EvaluateCondition($row[0]) -eq 1) {
        if ($session.DoAction('CraftRestoreLauncherFolder') -ne 1) { throw 'Launcher restore action failed.' }
      }
      $expected = if ($explicit) { $explicit } else { 'E:\Existing exact launcher' }
      if ((Get-SessionProperty $session 'INSTALLFOLDER') -ne $expected) { throw 'Launcher custom or existing path was lost.' }
    }
    $rows = @(Read-Rows $database 'SELECT `Root`, `Key`, `Name`, `Value` FROM `Registry`' 4)
    if (@($rows | Where-Object { $_[0] -ne '1' }).Count) { throw 'Launcher registration must stay under the current account.' }
    foreach ($entry in $policy.registry) {
      if (@($rows | Where-Object { $_[0] -eq $entry[0] -and $_[1] -eq $entry[1] -and $_[2] -eq $entry[2] -and $_[3] -eq $entry[3] }).Count -ne 1) {
        throw 'Launcher registration or shortcut preference changed unexpectedly.'
      }
    }
    Write-Output 'PASS ArtCraft Launcher: per-user scope, suite folder, remembered/custom paths and existing registration. No installation performed.'
    return
  }
  if ($properties['ALLUSERS'] -ne '2' -or $properties['MSIINSTALLPERUSER'] -ne '1') {
    throw 'Policy regression: new packages must default to the per-user dual-scope context.'
  }
  if ($properties['UpgradeCode'].Trim('{}') -ne $policy.upgradeCode) { throw 'The existing upgrade identity must not change.' }
  $directories = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Directory`, `Directory_Parent`, `DefaultDir` FROM `Directory`' 3)) {
    $directories[$row[0]] = @($row[1], $row[2])
  }
  if ($directories['INSTALLFOLDER'][0] -ne 'CraftSuiteFolder' -or
      $directories['CraftSuiteFolder'][1] -ne 'ArtCraft') { throw 'Expected the app under the ArtCraft suite folder.' }
  $programs = $directories['CraftSuiteFolder'][0]
  if ($programs -eq 'ProgramFiles6432Folder') { $programs = $directories[$programs][0] }
  if ($programs -notin 'ProgramFilesFolder', 'ProgramFiles64Folder') {
    throw 'Use Windows Installer native folder redirection, not a hard-coded profile path.'
  }
  $guards = @(Read-Rows $database 'SELECT `Condition`, `Description` FROM `LaunchCondition`' 2 |
    Where-Object { $_[0] -match 'CRAFT_(USER|MACHINE)_DIR' })
  if ($guards.Count -ne 2) { throw 'Expected both duplicate-installation guards.' }
  $cases = 0
  foreach ($scope in @('', '2', '1')) {
    foreach ($user in @('', 'D:\Existing user app')) {
      foreach ($machine in @('', 'E:\Existing machine app')) {
        foreach ($installed in @('', '1')) {
          Set-SessionProperty $session 'ALLUSERS' $scope
          Set-SessionProperty $session 'Installed' $installed
          Set-SessionProperty $session 'CRAFT_USER_DIR' $user
          Set-SessionProperty $session 'CRAFT_MACHINE_DIR' $machine
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
  # Execute the actual authored type-51 actions in a safe session, not copied resolver logic.
  foreach ($scope in @('', '2', '1')) {
    foreach ($explicit in @('', 'F:\Chosen exact app')) {
      Set-SessionProperty $session 'ALLUSERS' $scope
      Set-SessionProperty $session 'INSTALLFOLDER' $explicit
      Set-SessionProperty $session 'CRAFT_USER_DIR' 'D:\Existing user app'
      Set-SessionProperty $session 'CRAFT_MACHINE_DIR' 'E:\Existing machine app'
      $action = if ($scope -eq '1') { 'CraftRestoreMachineFolder' } else { 'CraftRestoreUserFolder' }
      # Evaluate the compiled condition, so the test exercises it instead of calling the action directly.
      $restore = @(Read-Rows $database ('SELECT `Condition` FROM `InstallExecuteSequence` WHERE `Action` = ''{0}''' -f $action) 1)[0]
      $selected = $session.EvaluateCondition($restore[0]) -eq 1
      if ($selected -eq [bool]$explicit) { throw "$action must run only when no folder was supplied (condition: $($restore[0]))." }
      if ($selected -and $session.DoAction($action) -ne 1) { throw 'Remembered-folder property action failed.' }
      $expected = if ($explicit) { $explicit } elseif ($scope -eq '1') { 'E:\Existing machine app' } else { 'D:\Existing user app' }
      if ((Get-SessionProperty $session 'INSTALLFOLDER') -ne $expected) { throw 'Custom or remembered folder was not preserved.' }
    }
  }
  $ui = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Action`, `Sequence` FROM `InstallUISequence`' 2)) { $ui[$row[0]] = [int]$row[1] }
  if (-not ($ui['AppSearch'] -lt $ui['CraftPreferMachineScope'] -and
      $ui['CraftPreferMachineScope'] -lt $ui['CraftScopeDlg'] -and
      $ui['CraftScopeDlg'] -lt $ui['FindRelatedProducts'] -and
      $ui['FindRelatedProducts'] -lt $ui['LaunchConditions'] -and
      $ui['CraftScopeDlg'] -lt $ui['CraftRestoreUserFolder'] -and $ui['CraftRestoreUserFolder'] -lt $ui['CostInitialize'])) {
    throw 'Scope must be chosen after AppSearch and before FindRelatedProducts, launch checks and costing.'
  }
  # Windows Installer resolves Program Files for the context it started in, before the wizard's scope choice.
  # The wizard therefore sets the new-install folder itself, and only in the wizard.
  $defaults = @(Read-Rows $database 'SELECT `Action`, `Sequence`, `Condition` FROM `InstallUISequence`' 3 | Where-Object { $_[0] -like 'CraftDefault*' })
  if ($defaults.Count -lt 2) { throw 'The wizard must set the new-install folder after the scope page; Program Files is resolved before the choice.' }
  foreach ($row in $defaults) {
    if (-not ($ui['CraftScopeDlg'] -lt [int]$row[1] -and [int]$row[1] -lt $ui['CostInitialize'])) { throw "$($row[0]) must run after the scope page and before costing." }
  }
  if (@(Read-Rows $database 'SELECT `Action` FROM `InstallExecuteSequence`' 1 | Where-Object { $_[0] -like 'CraftDefault*' }).Count) {
    throw 'Silent installs must not use the wizard-only default folder actions.'
  }
  # The machine default comes from the environment of the package architecture, not from a folder name.
  # x86 packages use Program Files (x86) on 64-bit Windows; x64 and Arm64 packages use the 64-bit root.
  $machineRoot = if ((Get-PackageTemplate $database) -match '^Intel') {
    if (Get-SessionProperty $session 'VersionNT64') { ${env:ProgramFiles(x86)} } else { $env:ProgramFiles }
  } elseif ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
  $machineFolder = (Join-Path $machineRoot "ArtCraft\$($policy.name)") + '\'
  foreach ($scope in @('1', '')) {
    Set-SessionProperty $session 'ALLUSERS' $scope
    Set-SessionProperty $session 'INSTALLFOLDER' ''
    Set-SessionProperty $session 'CRAFT_MACHINE_DIR' ''
    Set-SessionProperty $session 'CRAFT_USER_DIR' ''
    foreach ($row in @($defaults | Sort-Object { [int]$_[1] })) {
      if ($session.EvaluateCondition($row[2]) -eq 1 -and $session.DoAction($row[0]) -ne 1) { throw "$($row[0]) failed." }
    }
    $folder = Get-SessionProperty $session 'INSTALLFOLDER'
    $wanted = if ($scope -eq '1') { $machineFolder } else { (Join-Path $env:LOCALAPPDATA "Programs\ArtCraft\$($policy.name)") + '\' }
    if ($folder -ine $wanted) { throw "Wizard default folder for ALLUSERS='$scope' is '$folder', expected '$wanted'." }
  }
  $execute = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Action`, `Sequence` FROM `InstallExecuteSequence`' 2)) { $execute[$row[0]] = [int]$row[1] }
  foreach ($action in @('CraftRestoreUserFolder','CraftRestoreMachineFolder')) {
    if (-not ($execute['AppSearch'] -lt $execute[$action] -and $execute[$action] -lt $execute['CostInitialize'])) {
      throw 'Silent updates must restore the folder before directory costing.'
    }
  }
  foreach ($resolver in @('CraftUFromLegacyExe','CraftMFromLegacyExe')) {
    if (-not ($execute['AppSearch'] -lt $execute[$resolver] -and $execute[$resolver] -lt $execute['LaunchConditions'])) {
      throw 'Both legacy folder resolvers must run after AppSearch and before the launch checks.'
    }
  }
  # Switching the first-page choice must reset BOTH properties, including a back-and-forth change.
  $events = @(Read-Rows $database 'SELECT `Event`, `Argument`, `Condition`, `Ordering` FROM `ControlEvent` WHERE `Dialog_` = ''CraftScopeDlg'' AND `Control_` = ''Next'' ORDER BY `Ordering`' 4)
  # Model the full-UI upgrade entry with an existing machine-wide copy. Run
  # the actual preference action, then the compiled Next events before discovery.
  Set-SessionProperty $session 'ALLUSERS' '2'
  Set-SessionProperty $session 'CRAFT_SCOPE' 'user'
  Set-SessionProperty $session 'CRAFT_USER_DIR' ''
  Set-SessionProperty $session 'CRAFT_MACHINE_DIR' 'E:\Existing machine app'
  $preference = @(Read-Rows $database 'SELECT `Condition` FROM `InstallUISequence` WHERE `Action` = ''CraftPreferMachineScope''' 1)[0]
  if ($session.EvaluateCondition($preference[0]) -ne 1 -or $session.DoAction('CraftPreferMachineScope') -ne 1 -or
      (Get-SessionProperty $session 'CRAFT_SCOPE') -ne 'machine') {
    throw 'Full-UI machine upgrades must initially select the existing machine context.'
  }
  foreach ($choice in @('machine','user','machine','user')) {
    Set-SessionProperty $session 'CRAFT_SCOPE' $choice
    foreach ($event in $events) {
      if ($event[0] -notmatch '^\[([^\]]+)\]$') { continue }
      $property = $Matches[1]
      if ($session.EvaluateCondition($event[2]) -eq 1) {
        Set-SessionProperty $session $property $(if ($event[1] -eq '{}') { '' } else { $event[1] })
      }
    }
    $allUsers = Get-SessionProperty $session 'ALLUSERS'
    $perUser = Get-SessionProperty $session 'MSIINSTALLPERUSER'
    if (($choice -eq 'machine' -and ($allUsers -ne '1' -or $perUser)) -or
        ($choice -eq 'user' -and ($allUsers -or $perUser -ne '1'))) {
      throw 'Scope selector does not reset the requested installation context.'
    }
  }
  $actions = @(Read-Rows $database 'SELECT `Action`, `Source`, `Target`, `Type` FROM `CustomAction`' 4)
  $savedOptions = @($actions | Where-Object { $_[0] -match '^CraftLoadOption[0-9]+M$' })
  foreach ($option in $savedOptions) {
    if ([int]$option[3] -ne 51) { throw 'Saved-option test may run only property-setting actions.' }
    $number = [regex]::Match($option[0], '[0-9]+').Value
    foreach ($scope in @('2','1')) {
      Set-SessionProperty $session 'ALLUSERS' $scope
      Set-SessionProperty $session "CRAFT_OPTION_${number}_M" 'saved-machine-choice'
      Set-SessionProperty $session "CRAFT_OPTION_${number}_U" 'saved-user-choice'
      Set-SessionProperty $session $option[1] ''
      $action = "CraftLoadOption$number" + $(if ($scope -eq '1') { 'M' } else { 'U' })
      if ($session.DoAction($action) -ne 1) { throw 'Saved-option property action failed.' }
      $expected = if ($scope -eq '1') { 'saved-machine-choice' } else { 'saved-user-choice' }
      if ((Get-SessionProperty $session $option[1]) -ne $expected) { throw 'Saved installer choices leaked across contexts.' }
    }
  }
  # Follow the compiled control conditions, not the source's intended appearance.
  $controls = @(Read-Rows $database 'SELECT `Dialog_`, `Control`, `Type`, `Attributes`, `Text` FROM `Control`' 5)
  $conditions = @()
  if (@(Read-Rows $database 'SELECT `Name` FROM `_Tables` WHERE `Name` = ''ControlCondition''' 1).Count) {
    $conditions = @(Read-Rows $database 'SELECT `Dialog_`, `Control_`, `Action`, `Condition` FROM `ControlCondition`' 4)
  }
  function Test-ControlVisible($Control) {
    $visible = ([int]$Control[3] -band 1) -ne 0
    foreach ($condition in $conditions) {
      if ($condition[0] -eq $Control[0] -and $condition[1] -eq $Control[1] -and $session.EvaluateCondition($condition[3]) -eq 1) {
        if ($condition[2] -eq 'Show') { $visible = $true }
        if ($condition[2] -eq 'Hide') { $visible = $false }
      }
    }
    return $visible
  }
  foreach ($button in @($controls | Where-Object { $_[0] -in 'InstallWelcome','InstallMaintenance','DesktopShortcutDlg' -and ([int]$_[3] -band 8388608) -ne 0 })) {
    $plain = @($controls | Where-Object { $_[0] -eq $button[0] -and $_[1] -eq ($button[1] + 'User') })
    if ($plain.Count -ne 1 -or ([int]$plain[0][3] -band 8388608)) { throw 'Scope-aware buttons need an unshielded per-user counterpart.' }
    foreach ($scope in @('', '2', '1')) {
      Set-SessionProperty $session 'ALLUSERS' $scope
      if ((Test-ControlVisible $button) -ne ($scope -eq '1') -or (Test-ControlVisible $plain[0]) -ne ($scope -ne '1')) {
        throw 'The elevation shield must follow the selected scope.'
      }
    }
    $dialog = $button[0]; $shieldId = $button[1]; $plainId = $plain[0][1]
    $shieldEvents = @(Read-Rows $database "SELECT ``Event``, ``Argument``, ``Condition``, ``Ordering`` FROM ``ControlEvent`` WHERE ``Dialog_`` = '$dialog' AND ``Control_`` = '$shieldId' ORDER BY ``Ordering``" 4 | ForEach-Object { $_ -join '|' })
    $plainEvents = @(Read-Rows $database "SELECT ``Event``, ``Argument``, ``Condition``, ``Ordering`` FROM ``ControlEvent`` WHERE ``Dialog_`` = '$dialog' AND ``Control_`` = '$plainId' ORDER BY ``Ordering``" 4 | ForEach-Object { $_ -join '|' })
    if (($shieldEvents -join "`n") -ne ($plainEvents -join "`n")) { throw 'Per-user and machine buttons must perform the same operation.' }
  }
  foreach ($scope in @('', '2', '1')) {
    Set-SessionProperty $session 'ALLUSERS' $scope
    foreach ($text in @($controls | Where-Object { $_[4] -like '*for all users*' })) {
      if ((Test-ControlVisible $text) -ne ($scope -eq '1')) { throw 'The welcome-page scope text must follow the selected scope.' }
    }
  }
  if ($policy.id -in 'deckcraft','filmcraft','soundcraft') {
    foreach ($route in @(@('WelcomeDlg','Next','InstallDirDlg'),@('InstallDirDlg','Back','WelcomeDlg'))) {
      $dialog = $route[0]; $control = $route[1]
      $routes = @(Read-Rows $database "SELECT ``Argument``, ``Condition`` FROM ``ControlEvent`` WHERE ``Dialog_`` = '$dialog' AND ``Control_`` = '$control' AND ``Event`` = 'NewDialog' ORDER BY ``Ordering``" 2)
      Set-SessionProperty $session 'Installed' ''
      $destination = ''
      foreach ($event in $routes) { if ($session.EvaluateCondition($event[1]) -eq 1) { $destination = $event[0] } }
      if ($destination -ne $route[2]) { throw 'Skipping the licence page must work in both navigation directions.' }
    }
  }
  $registry = @(Read-Rows $database 'SELECT `Root`, `Key`, `Name`, `Value` FROM `Registry`' 4)
  if (@($registry | Where-Object { $_[0] -in '1', '2' }).Count) { throw 'Registration must follow the selected context, not force HKCU or HKLM.' }
  if (@($registry | Where-Object { $_[0] -eq '0' -and $_[1] -match '^Software\\Classes(\\|$)' }).Count) {
    throw 'HKCR keys must be relative to Classes.'
  }
  foreach ($entry in $policy.registry) {
    $found = @($registry | Where-Object { $_[0] -eq $entry[0] -and $_[1] -eq $entry[1] -and $_[2] -eq $entry[2] -and $_[3] -eq $entry[3] })
    if ($found.Count -ne 1) { throw "Lost or changed registration: $($entry[1]) / $($entry[2])" }
  }
  $installDir = @($registry | Where-Object { $_[0] -eq '-1' -and $_[1] -eq "Software\$($policy.name)" -and $_[2] -eq 'InstallDir' -and $_[3] -eq '[INSTALLFOLDER]' })
  if ($installDir.Count -ne 1) { throw 'A context-sensitive installation-folder record is required.' }
  Write-Output "PASS $($policy.name): $cases scope cases, folder preservation, action sequencing and registry contract. No installation performed."
} finally {
  foreach ($com in @($session, $database, $installer)) {
    if ($null -ne $com) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($com) }
  }
}
