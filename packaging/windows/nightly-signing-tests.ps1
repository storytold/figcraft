param(
  [Parameter(Mandatory=$true)][string] $FixtureRoot,
  [string] $PackagingRoot = $PSScriptRoot,
  [switch] $PhaseGuardOnly
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PackagingRoot 'nightly.ps1') -FunctionsOnly
$sign = Join-Path $PackagingRoot 'sign.ps1'
$names = @('WINDOWS_CERTIFICATE','WINDOWS_CERTIFICATE_PASSWORD','AZURE_TENANT_ID','AZURE_CLIENT_ID','AZURE_CLIENT_SECRET','AZURE_SIGNING_ENDPOINT','AZURE_SIGNING_ACCOUNT','AZURE_CERT_PROFILE')
function Clear-SigningFixtures { foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $null) } }
function Assert-Policy([bool] $Value, [string] $Message) { if (-not $Value) { throw $Message } }
function Invoke-Refusal([scriptblock] $Operation, [string] $Message) {
  $errorText = ''
  try { & $Operation | Out-Null } catch { $errorText = $_.Exception.Message }
  Assert-Policy ($errorText.Contains($Message)) "expected refusal: $Message; got: $errorText"
}
function global:cargo { $global:SigningFixtureCargoCalls++; throw 'FIXTURE_COMPILER_BOUNDARY' }
function global:Invoke-WebRequest { throw 'fixture must not perform signing downloads' }
$global:SigningFixtureCargoCalls = 0
if (-not [IO.Path]::IsPathRooted($FixtureRoot)) { throw 'use an owned absolute FixtureRoot' }
$root = Join-Path $FixtureRoot ('signing-policy-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
# The fixture mocks Cargo, but nightly.ps1 checks the same source identity a packaging run does:
# the version names this checkout's commit and the run's UTC date, and the tree is clean. Deriving
# them here keeps the fixture reaching the mocked compiler instead of failing in source validation.
$sha = "$(git -C $PSScriptRoot rev-parse HEAD)".Trim()
$runDate = (Get-Date).ToUniversalTime().ToString('yyyyMMdd')
$version = "0.18.1-nightly.$runDate.12345.1.g$($sha.Substring(0, 12))"
$env:FIGCRAFT_VERSION = $version
$env:FIGCRAFT_BUILD_SHA = $sha
$env:FIGCRAFT_BUILD_DATE = "$($runDate.Substring(0, 4))-$($runDate.Substring(4, 2))-$($runDate.Substring(6, 2))"
function Invoke-Build([switch] $BuildOnly) {
  $arguments = @{Arch='x64';Version=$version;Target='x86_64-pc-windows-msvc';TargetDir=$root;Dist=(Join-Path $root 'dist')}
  if ($BuildOnly) { $arguments.BuildOnly = $true }
  & (Join-Path $PackagingRoot 'nightly.ps1') @arguments
}
try {
  Clear-SigningFixtures
  Invoke-Refusal { Invoke-Build } 'requires -BuildOnly'
  Assert-Policy ($global:SigningFixtureCargoCalls -eq 0) 'default mode invoked Cargo'
  Write-Output 'PASS: explicit-build-phase-required'
  if ($PhaseGuardOnly) { return }

  foreach ($name in @('WINDOWS_CERTIFICATE','WINDOWS_CERTIFICATE_PASSWORD','AZURE_CLIENT_SECRET')) {
    Clear-SigningFixtures
    [Environment]::SetEnvironmentVariable($name, 'generated-fixture-not-a-secret')
    Invoke-Refusal { Invoke-Build -BuildOnly } 'Compilation must not receive signing credentials'
    Assert-Policy ($global:SigningFixtureCargoCalls -eq 0) 'key-bearing compilation reached Cargo'
  }
  Write-Output 'PASS: credentials-rejected-before-compilation'

  Clear-SigningFixtures
  Invoke-Refusal { Invoke-Build -BuildOnly } 'FIXTURE_COMPILER_BOUNDARY'
  Assert-Policy ($global:SigningFixtureCargoCalls -eq 1) 'clean compilation did not reach the mocked compiler boundary'
  Assert-Policy (-not (Test-Path (Join-Path $root 'dist'))) 'mocked compilation produced an artifact'
  Write-Output 'PASS: secret-free-build-reaches-only-mocked-boundary'

  Clear-SigningFixtures
  Invoke-Refusal { & $sign -Required -CheckOnly } 'Nightly signing is required'
  $env:AZURE_CLIENT_SECRET = 'generated-partial-fixture'
  Invoke-Refusal { & $sign -Required -CheckOnly } 'Nightly signing is required'
  Write-Output 'PASS: required-signer-refuses-missing-and-partial-configuration'

  Clear-SigningFixtures
  $env:WINDOWS_CERTIFICATE = 'generated-presence-fixture-not-a-certificate'
  & $sign -Required -CheckOnly
  Clear-SigningFixtures
  foreach ($name in $names | Where-Object { $_.StartsWith('AZURE_') }) { [Environment]::SetEnvironmentVariable($name, 'generated-presence-fixture') }
  & $sign -Required -CheckOnly
  Clear-SigningFixtures
  Write-Output 'PASS: supported-configuration-presence-without-signing'

  # Load the policy function without signing. These objects are not crypto evidence.
  . $sign -CheckOnly
  foreach ($value in @($null, [pscustomobject]@{Status='NotSigned';SignerCertificate='fixture'}, [pscustomobject]@{Status='HashMismatch';SignerCertificate='fixture'}, [pscustomobject]@{Status='Valid';SignerCertificate=$null})) {
    Invoke-Refusal { Assert-AuthenticodeSignature $value } 'valid Authenticode signature'
  }
  Assert-AuthenticodeSignature ([pscustomobject]@{Status='Valid';SignerCertificate='generated-public-certificate-placeholder'})
  Write-Output 'PASS: required-signature-policy-rejects-invalid-or-missing-signer'

  $file = Join-Path $root 'owned-sentinel.exe'
  [IO.File]::WriteAllText($file, 'generated sentinel, not a real executable')
  $before = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash
  & $sign -Files $file
  Assert-Policy ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ceq $before) 'optional stable policy changed unsigned bytes'
  Write-Output 'PASS: stable-optional-policy-preserves-owned-bytes'
} finally {
  Clear-SigningFixtures
  Remove-Item -LiteralPath $root -Recurse -Force
}
