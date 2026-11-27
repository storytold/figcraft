param([Parameter(Mandatory=$true)][string] $FixtureRoot)
. (Join-Path $PSScriptRoot 'nightly.ps1') -FunctionsOnly
function Assert([bool] $Value, [string] $Message) { if (-not $Value) { throw $Message } }
$spec = Get-NightlyArchiveSpec '0.18.1-nightly.20261124.17.1.gabcdef012345' 'x64'
Assert ($spec.Archive -eq 'figcraft-0.18.1-nightly.20261124.17.1.gabcdef012345-windows-x86_64-nightly.zip') 'wrong nightly artifact name'
Assert ($spec.Executables.Count -eq 1 -and $spec.Executables[0] -eq 'figcraft.exe') 'nightly must not ship the CLI'
Assert ($spec.AppId -eq 'ai.storyteller.figcraft.nightly-v1') 'nightly identity is not distinct'
foreach ($arch in @('x86', 'arm64')) {
  $refused = $false
  try { Get-NightlyArchiveSpec '0.18.1' $arch | Out-Null } catch { $refused = $true }
  Assert $refused 'unsupported target was accepted'
}
foreach ($bad in @('', '0.18.1', '../other', '0.18.1/elsewhere')) {
  $refused = $false
  try { Get-NightlyArchiveSpec $bad 'x64' | Out-Null } catch { $refused = $true }
  Assert $refused 'unsafe artifact version was accepted'
}
Assert-NightlyFlavor 'FigCraft Nightly' 'FigCraft 0.18.1-nightly.20261124.17.1.gabcdef012345 (nightly-v1)' '0.18.1-nightly.20261124.17.1.gabcdef012345'
foreach ($pair in @(@('FigCraft', 'FigCraft 0.18.1 (nightly-v1)'), @('FigCraft Nightly', 'FigCraft 0.18.1'))) {
  $refused = $false
  try { Assert-NightlyFlavor $pair[0] $pair[1] '0.18.1-nightly.20261124.17.1.gabcdef012345' } catch { $refused = $true }
  Assert $refused 'stable or mismatched build flavor was accepted'
}
$sha = 'abcdef0123456789abcdef0123456789abcdef01'
$good = '0.18.1-nightly.20261124.17.1.gabcdef012345'
Assert-NightlySource $good $sha '2026-11-24' $sha ''
$cases = @(
  @('missing commit', $good, '', '2026-11-24', $sha, ''),
  @('short commit', $good, 'abcdef012345', '2026-11-24', $sha, ''),
  @('uppercase commit', $good, $sha.ToUpper(), '2026-11-24', $sha.ToUpper(), ''),
  @('version names another commit', '0.18.1-nightly.20261124.17.1.g000000000000', $sha, '2026-11-24', $sha, ''),
  @('missing date', $good, $sha, '', $sha, ''),
  @('date differs from version', $good, $sha, '2026-11-25', $sha, ''),
  @('other commit checked out', $good, $sha, '2026-11-24', ('0' * 40), ''),
  @('dirty tree', $good, $sha, '2026-11-24', $sha, ' M apps/figcraft/src/main.rs')
)
foreach ($case in $cases) {
  $refused = $false
  try { Assert-NightlySource $case[1] $case[2] $case[3] $case[4] $case[5] } catch { $refused = $true }
  Assert $refused "source identity accepted: $($case[0])"
}
Write-Output 'nightly packaging spec, target/version refusal, source identity and flavor-binding tests passed'

# Exercise the SAME production notice copier with the real repository layout, then inspect a ZIP.
if (-not $FixtureRoot -or -not [IO.Path]::IsPathRooted($FixtureRoot)) { throw 'supply an owned absolute -FixtureRoot for the notices/ZIP regression' }
$testRoot = Join-Path $FixtureRoot ("nightly-notices-" + [Guid]::NewGuid().ToString('N'))
try {
  $repo = Join-Path $testRoot 'app'
  $fontRepo = Join-Path $testRoot 'craft-fonts'
  $family = Join-Path $fontRepo 'fonts\ExampleFamily'
  $stage = Join-Path $testRoot 'stage'
  New-Item -ItemType Directory -Force -Path $repo, $family, $stage | Out-Null
  $rootNames = @('README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ATTRIBUTION.md')
  foreach ($name in $rootNames) { Set-Content -LiteralPath (Join-Path $repo $name) -Value "root-notice-$name" }
  Set-Content -LiteralPath (Join-Path $family 'OFL.txt') -Value 'font-notice-example'
  Copy-NightlyNotices $repo $fontRepo $stage
  foreach ($name in @($rootNames) + @('OFL-ExampleFamily.txt')) {
    Assert (Test-Path -LiteralPath (Join-Path $stage $name)) "missing staged notice $name"
  }
  $zip = Join-Path $testRoot 'notices.zip'
  Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
  $archive = [IO.Compression.ZipFile]::OpenRead($zip)
  try {
    $names = @($archive.Entries | ForEach-Object { $_.FullName })
    foreach ($name in @($rootNames) + @('OFL-ExampleFamily.txt')) { Assert ($name -in $names) "missing archived notice $name" }
    foreach ($forbidden in @('portable', 'figcraft-cli.exe')) { Assert ($forbidden -notin $names) "unexpected payload $forbidden" }
    $entry = $archive.GetEntry('OFL-ExampleFamily.txt')
    $reader = [IO.StreamReader]::new($entry.Open())
    try { Assert ($reader.ReadToEnd().Contains('font-notice-example')) 'wrong font license bytes in ZIP' } finally { $reader.Dispose() }
  } finally { $archive.Dispose() }
} finally {
  if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
Write-Output 'nightly mock-repository root and font notices are present in stage and ZIP'

# The source state comes from git itself: the checked-out commit, then a dirty tree.
$gitRoot = Join-Path $FixtureRoot ("nightly-source-" + [Guid]::NewGuid().ToString('N'))
try {
  New-Item -ItemType Directory -Force -Path $gitRoot | Out-Null
  git -C $gitRoot init -q; git -C $gitRoot -c user.name=fixture -c user.email=fixture@invalid commit -q --allow-empty -m fixture
  if ($LASTEXITCODE -ne 0) { throw 'cannot create the git fixture' }
  $head = (git -C $gitRoot rev-parse HEAD).Trim()
  $state = Get-NightlySourceState $gitRoot
  Assert ($state.Head -ceq $head -and -not $state.Status) 'clean checkout not reported as clean at HEAD'
  $version = '0.18.1-nightly.20261124.17.1.g' + $head.Substring(0, 12)
  Assert-NightlySource $version $head '2026-11-24' $state.Head $state.Status
  Set-Content -LiteralPath (Join-Path $gitRoot 'untracked.txt') -Value 'dirty'
  $state = Get-NightlySourceState $gitRoot
  $refused = $false
  try { Assert-NightlySource $version $head '2026-11-24' $state.Head $state.Status } catch { $refused = $true }
  Assert $refused 'a dirty checkout was accepted'
} finally {
  if (Test-Path -LiteralPath $gitRoot) { Remove-Item -LiteralPath $gitRoot -Recurse -Force }
}
Write-Output 'nightly source state is read from git and a dirty checkout is refused'
