<# First nightly target only: Windows x86_64 GUI ZIP. No MSI, CLI, portable marker or association registration. #>
param(
  [string] $Arch = 'x64',
  [string] $Version,
  [string] $Target,
  [string] $TargetDir,
  [string] $Dist,
  [switch] $SkipBuild,
  [switch] $BuildOnly,
  [switch] $FunctionsOnly
)
$ErrorActionPreference = 'Stop'

function Get-NightlyArchiveSpec([string] $Version, [string] $Arch) {
  if ($Arch -ne 'x64') { throw 'nightly-v1 supports only Windows x86_64' }
  if (-not $Version -or $Version -cnotmatch '^\d+\.\d+\.\d+-nightly\.\d{8}\.\d+\.\d+\.g[0-9a-f]{12}\z') { throw 'invalid nightly artifact version' }
  return @{
    Archive = "figcraft-$Version-windows-x86_64-nightly.zip"
    UnsignedArchive = "figcraft-$Version-windows-x86_64-nightly.unsigned.zip"
    Executables = @('figcraft.exe')
    Channel = 'nightly-v1'
    AppId = 'ai.storyteller.figcraft.nightly-v1'
  }
}

# The same source identity the Linux helper enforces (nightly-common.sh): the version names the
# commit being built, the date is the original run's, and the tree is clean.
function Assert-NightlySource([string] $Version, [string] $Sha, [string] $BuildDate, [string] $Head, [string] $Status) {
  if ($Sha -cnotmatch '^[0-9a-f]{40}\z') { throw 'an exact source commit is required (FIGCRAFT_BUILD_SHA)' }
  if ($Version -cnotmatch '^\d+\.\d+\.\d+-nightly\.\d{8}\.\d+\.\d+\.g[0-9a-f]{12}\z') { throw 'invalid nightly artifact version' }
  if ($Version.Substring($Version.Length - 12) -cne $Sha.Substring(0, 12)) { throw 'version/source commit mismatch' }
  if ($BuildDate -cnotmatch '^\d{4}-\d{2}-\d{2}\z') { throw 'the original run-created UTC date is required (FIGCRAFT_BUILD_DATE)' }
  $versionDate = ($Version -split '-nightly\.', 2)[1].Substring(0, 8)
  if ($BuildDate.Replace('-', '') -cne $versionDate) { throw 'version/original-run date mismatch' }
  if ($Head -cne $Sha) { throw 'prepared source is not the checked-out commit' }
  if ($Status) { throw 'nightly packaging requires the frozen clean source' }
}

function Get-NightlySourceState([string] $RepositoryRoot) {
  $head = git -C $RepositoryRoot rev-parse HEAD
  if ($LASTEXITCODE -ne 0) { throw 'cannot read the checked-out commit' }
  $status = git -C $RepositoryRoot status --porcelain
  if ($LASTEXITCODE -ne 0) { throw 'cannot read the source tree status' }
  return @{ Head = "$head".Trim(); Status = ($status | Out-String).Trim() }
}

function Assert-NightlyFlavor([string] $ProductName, [string] $VersionOutput, [string] $ExpectedVersion) {
  if ($ProductName -ne 'FigCraft Nightly' -or $VersionOutput -cne "FigCraft $ExpectedVersion (nightly-v1)") {
    throw 'the executable is not the compiled nightly-v1 flavor'
  }
}

function Copy-NightlyNotices([string] $RepositoryRoot, [string] $FontRepository, [string] $Destination) {
  foreach ($name in 'README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ATTRIBUTION.md') {
    $source = Join-Path $RepositoryRoot $name
    if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $Destination }
  }
  if ($FontRepository) {
    $fonts = Join-Path $FontRepository 'fonts'
    if (-not (Test-Path -LiteralPath $fonts -PathType Container)) { throw 'CRAFT_FONTS_DIR has no fonts directory for the bundled notices' }
    foreach ($family in Get-ChildItem -LiteralPath $fonts -Directory) {
      $license = Join-Path $family.FullName 'OFL.txt'
      if (Test-Path -LiteralPath $license) { Copy-Item -LiteralPath $license -Destination (Join-Path $Destination "OFL-$($family.Name).txt") }
    }
  }
}

if ($FunctionsOnly) { return }
# The prepared version comes from the nightly callers: `FIGCRAFT_VERSION` (packaging/env.sh) or the
# override `packaging/nightly-common.sh` hands the compiler. The Cargo version is never a fallback.
$Prepared = if ($env:FIGCRAFT_OVERRIDE_VERSION) { $env:FIGCRAFT_OVERRIDE_VERSION } else { $env:FIGCRAFT_VERSION }
if (-not $Prepared -or $Version -cne $Prepared) { throw 'nightly-v1 requires explicit FIGCRAFT_VERSION, not the Cargo version fallback' }
$Spec = Get-NightlyArchiveSpec $Version $Arch
if ($SkipBuild) { throw 'nightly-v1 forbids -SkipBuild; the executable flavor must be built in this operation' }
if ($Target -ne 'x86_64-pc-windows-msvc') { throw 'nightly-v1 target must be x86_64-pc-windows-msvc' }
if (-not $BuildOnly) { throw 'Nightly compilation requires -BuildOnly; final archives are produced in the separate trusted signing job' }
# No key values may enter compilation. This produces an internal intermediate, not a feed artifact.
& (Join-Path $PSScriptRoot 'sign.ps1') -CheckBuildEnvironment
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Source = Get-NightlySourceState $Root
Assert-NightlySource $Version $env:FIGCRAFT_BUILD_SHA $env:FIGCRAFT_BUILD_DATE $Source.Head $Source.Status
$oldChannel = $env:FIGCRAFT_BUILD_CHANNEL
$oldVersionOverride = $env:FIGCRAFT_OVERRIDE_VERSION
try {
  $env:FIGCRAFT_BUILD_CHANNEL = 'nightly-v1'
  $env:FIGCRAFT_OVERRIDE_VERSION = $Version
  $env:FIGCRAFT_REQUIRE_WINRES = '1'
  $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
  cargo build --release --locked -p figcraft --target $Target
  if ($LASTEXITCODE -ne 0) { throw "nightly cargo build failed: $LASTEXITCODE" }
} finally {
  $env:FIGCRAFT_BUILD_CHANNEL = $oldChannel
  $env:FIGCRAFT_OVERRIDE_VERSION = $oldVersionOverride
}
$Source = Get-NightlySourceState $Root
try { Assert-NightlySource $Version $env:FIGCRAFT_BUILD_SHA $env:FIGCRAFT_BUILD_DATE $Source.Head $Source.Status }
catch { throw "source changed during the build: $_" }
$Exe = Join-Path $TargetDir "$Target\release\figcraft.exe"
$pe = [System.IO.File]::ReadAllBytes($Exe)
if ($pe.Length -lt 64) { throw 'truncated nightly PE executable' }
$offset = [BitConverter]::ToInt32($pe, 0x3C)
if ($offset -lt 0 -or $offset -gt $pe.Length - 0x5E) { throw 'invalid nightly PE header offset' }
if ([BitConverter]::ToUInt16($pe, $offset + 4) -ne 0x8664 -or [BitConverter]::ToUInt16($pe, $offset + 0x5C) -ne 2) {
  throw 'nightly executable must be x86_64 Windows GUI'
}
$VersionInfo = (Get-Item -LiteralPath $Exe).VersionInfo
$ProductName = $VersionInfo.ProductName
if ($VersionInfo.ProductVersion -cne $Version) { throw 'nightly PE ProductVersion differs from the feed identity' }
# Check the embedded build flavor before invoking even the side-effect-free version query.
if ($ProductName -ne 'FigCraft Nightly') { throw 'nightly executable is missing the embedded build flavor' }
$VersionOutput = (& $Exe --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw 'nightly --version query failed' }
Assert-NightlyFlavor $ProductName $VersionOutput $Version
$Stage = Join-Path $TargetDir 'windows-package\nightly-v1-x86_64'
if (Test-Path -LiteralPath $Stage) { Remove-Item -LiteralPath $Stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item -LiteralPath $Exe -Destination (Join-Path $Stage 'figcraft.exe')
# Signing happens only in the separate trusted step, using sign.ps1 -Required.
# The intermediate's filename cannot satisfy the feed-ready archive contract.
Copy-NightlyNotices $Root $env:CRAFT_FONTS_DIR $Stage
if ((Get-ChildItem -LiteralPath $Stage -Filter '*.exe').Count -ne 1 -or (Test-Path (Join-Path $Stage 'portable'))) {
  throw 'nightly stage contains an unexpected executable or portable fallback marker'
}
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
$Zip = Join-Path $Dist $Spec.UnsignedArchive
if (Test-Path -LiteralPath $Zip) { throw 'Nightly output already exists; refusing to replace prepared artifact bytes' }
Compress-Archive -Path (Join-Path $Stage '*') -DestinationPath $Zip
$Archive = [System.IO.Compression.ZipFile]::OpenRead($Zip)
try {
  $Names = @($Archive.Entries | ForEach-Object { $_.FullName })
  if ('figcraft.exe' -notin $Names -or 'figcraft-cli.exe' -in $Names -or 'portable' -in $Names) {
    throw 'nightly ZIP payload violates the GUI-only profile contract'
  }
} finally { $Archive.Dispose() }
Write-Output "Nightly build intermediate (not publishable): $Zip ($VersionOutput)"
