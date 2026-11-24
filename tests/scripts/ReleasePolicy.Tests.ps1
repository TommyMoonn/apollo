#requires -Version 7.0
# Optional local release-policy check; writes only to disposable fixtures.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$validator = Join-Path $repo 'scripts/check-release.ps1'
$updater = Join-Path $repo 'scripts/set-version.ps1'
. $validator

function Assert-Check([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Rejected([scriptblock]$Action, [string]$Case) {
    try { & $Action | Out-Null } catch { return }
    throw "Unexpectedly accepted: $Case"
}

$fixture = Join-Path ([IO.Path]::GetTempPath()) ('apollo-release-policy-' + [guid]::NewGuid().ToString('N'))
$environment = @{}
foreach ($name in @('GITHUB_REF', 'GITHUB_REF_TYPE', 'GITHUB_REF_NAME', 'CARGO_NET_OFFLINE')) {
    $environment[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
    foreach ($name in @('GITHUB_REF', 'GITHUB_REF_TYPE', 'GITHUB_REF_NAME')) {
        [Environment]::SetEnvironmentVariable($name, $null)
    }
    $env:CARGO_NET_OFFLINE = 'true'
    $current = Invoke-ReleaseValidation -ProjectRoot $repo
    $null = Invoke-ReleaseValidation -ProjectRoot $repo -Tag "v$($current.Version)" -RequireChangelog
    Write-Host 'PASS current repository metadata and canonical note'

    $null = New-Item -ItemType Directory -Path $fixture, (Join-Path $fixture 'src'), (Join-Path $fixture 'dep/src'), (Join-Path $fixture 'release-notes') -Force
    $manifestPath = Join-Path $fixture 'Cargo.toml'
    $lockPath = Join-Path $fixture 'Cargo.lock'
    $changelogPath = Join-Path $fixture 'CHANGELOG.md'
    $notePath = Join-Path $fixture 'release-notes/v1.2.3.md'
    $manifest = @'
[package]
name = "apollo"
version = "1.2.3"
edition = "2024"
[dependencies]
fixture-dep = { path = "dep" }
'@
    [IO.File]::WriteAllText($manifestPath, "$manifest`n")
    [IO.File]::WriteAllText((Join-Path $fixture 'src/main.rs'), 'fn main() {}')
    [IO.File]::WriteAllText((Join-Path $fixture 'dep/Cargo.toml'), "[package]`nname = `"fixture-dep`"`nversion = `"2.3.4`"`nedition = `"2024`"`n")
    [IO.File]::WriteAllText((Join-Path $fixture 'dep/src/lib.rs'), '')
    & cargo generate-lockfile --offline --manifest-path $manifestPath
    if ($LASTEXITCODE -ne 0) { throw 'Could not generate the fixture lockfile.' }
    $originalLock = [IO.File]::ReadAllText($lockPath)
    $originalManifest = [IO.File]::ReadAllText($manifestPath)
    $hashes = { @(Get-FileHash -LiteralPath $manifestPath, $lockPath | ForEach-Object Hash) -join ':' }
    $originalHashes = & $hashes

    & $validator --project $fixture
    Assert-Check ((& $hashes) -ceq $originalHashes) 'Metadata validation modified version files.'
    Assert-Rejected { Invoke-ReleaseValidation -ProjectRoot $fixture -Tag 'v1.2.30' } 'partial tag'
    Assert-Rejected { Invoke-ReleaseValidation -ProjectRoot $fixture -RequireChangelog } 'candidate without tag'
    foreach ($version in @('0.0.0', '65535.65535.65535')) { Assert-StableReleaseVersion $version }
    foreach ($version in @('01.2.3', '1.2', '1.2.3-rc.1', '1.2.3+build', '65536.0.0', '1.65536.0', '1.2.65536', '999999999999999999.0.0')) {
        Assert-Rejected { Assert-StableReleaseVersion $version } $version
    }
    foreach ($badLock in @(
        $originalLock.Replace('version = "1.2.3"', 'version = "1.2.4"'),
        ($originalLock + "`n[[package]]`nname = `"apollo`"`nversion = `"1.2.3`"`n"),
        $originalLock.Replace('version = "1.2.3"', "version = `"1.2.3`"`nversion = `"1.2.3`"")
    )) {
        [IO.File]::WriteAllText($lockPath, $badLock)
        Assert-Rejected { Invoke-ReleaseValidation -ProjectRoot $fixture } 'invalid root lock entry'
    }
    [IO.File]::WriteAllText($lockPath, $originalLock)
    $env:GITHUB_REF_TYPE = 'tag'
    $env:GITHUB_REF_NAME = 'v1.2.3'
    $env:GITHUB_REF = 'refs/tags/v1.2.3'
    Assert-Check ((Invoke-ReleaseValidation -ProjectRoot $fixture).Tag -ceq 'v1.2.3') 'Inherited tag not used.'
    Assert-Rejected { Invoke-ReleaseValidation -ProjectRoot $fixture -Tag 'v1.2.4' } 'explicit tag conflict'
    $env:GITHUB_REF = 'refs/tags/v1.2.4'
    Assert-Rejected { Invoke-ReleaseValidation -ProjectRoot $fixture } 'inherited tag conflict'
    foreach ($name in @('GITHUB_REF', 'GITHUB_REF_TYPE', 'GITHUB_REF_NAME')) {
        [Environment]::SetEnvironmentVariable($name, $null)
    }
    Write-Host 'PASS stable version bounds, lock identity, exact tags and read-only validation'

    $heading = '## [1.2.3] - 2028-02-29'
    $link = '[1.2.3]: https://github.com/TommyMoonn/apollo/compare/v1.2.2...v1.2.3'
    $changelog = "$heading`n`n$link`n"
    $note = @'
## Downloads

- [Installer](https://github.com/TommyMoonn/apollo/releases/download/v1.2.3/Apollo-Setup-x64.exe)
- [Portable](https://github.com/TommyMoonn/apollo/releases/download/v1.2.3/Apollo-windows-x64.zip)

## Changes

- Preserve the user's media.
'@
    $note = ($note -replace '\r?\n', "`r`n") + "`r`n`r`n"
    $candidate = { Invoke-ReleaseValidation -ProjectRoot $fixture -Tag 'v1.2.3' -RequireChangelog }
    [IO.File]::WriteAllText($changelogPath, $changelog)
    Assert-Rejected $candidate 'missing note'
    [IO.File]::WriteAllText($notePath, $note)
    $result = & $candidate
    Assert-Check ($result.Note.Body -ceq $note -and $result.Note.Date -ceq '2028-02-29') 'Canonical body or date changed during parsing.'
    & $validator --project $fixture --tag v1.2.3 --require-changelog
    foreach ($invalid in @(
        "$link`n", "$heading`n", "$heading`n$changelog", "$changelog$link`n",
        $changelog.Replace('2028-02-29', '2027-02-29'), $changelog.Replace('...v1.2.3', '...v1.2.30')
    )) {
        [IO.File]::WriteAllText($changelogPath, $invalid)
        Assert-Rejected $candidate 'invalid changelog'
    }
    [IO.File]::WriteAllText($changelogPath, $changelog)
    foreach ($invalid in @(
        '', $note.Replace('## Changes', '## Other'), $note.Replace("- Preserve the user's media.", ''),
        $note.Replace('/v1.2.3/', '/v1.2.30/'), $note.Replace('Apollo-windows-x64.zip', 'Apollo-Setup-x64.exe'),
        $note.Replace('TommyMoonn/apollo', 'other/apollo'), $note.Replace("Preserve the user's media.", '{{change}}'),
        "$note## Changes`n- Duplicate", "# v1.2.3`n$note"
    )) {
        [IO.File]::WriteAllText($notePath, $invalid)
        Assert-Rejected $candidate 'invalid release note'
    }
    [IO.File]::WriteAllText($notePath, $note)
    Write-Host 'PASS candidate changelog, exact asset destinations and unchanged note body'

    & $updater 1.2.4 --project $fixture
    $expectedLock = $originalLock.Replace('version = "1.2.3"', 'version = "1.2.4"')
    Assert-Check (([IO.File]::ReadAllText($lockPath) -replace '\r\n', "`n") -ceq ($expectedLock -replace '\r\n', "`n")) 'Unrelated lockfile content changed.'
    Assert-Check (([IO.File]::ReadAllText($manifestPath)) -ceq $originalManifest.Replace('version = "1.2.3"', 'version = "1.2.4"')) 'Manifest change was not version-only.'
    $updatedHashes = & $hashes
    & $updater 1.2.4 --project $fixture
    Assert-Check ((& $hashes) -ceq $updatedHashes) 'No-op version update modified files.'

    # Substitute only Cargo's failure/side effect; exercise actual rollback.
    $receipt = @{ Calls = 0 }
    & {
        function cargo {
            $receipt.Calls++
            [IO.File]::AppendAllText($lockPath, "`n# interrupted Cargo operation`n")
            $global:LASTEXITCODE = 1
        }
        Assert-Rejected { & $updater 1.2.5 --project $fixture } 'failed Cargo update'
    }
    Assert-Check ($receipt.Calls -eq 1) 'Rollback fixture did not reach the failing Cargo operation.'
    Assert-Check ((& $hashes) -ceq $updatedHashes) 'Failed Cargo update did not restore both files.'
    $receipt.Calls = 0
    & {
        function cargo {
            $receipt.Calls++
            if ($args[0] -eq 'update') {
                $lock = [IO.File]::ReadAllText($lockPath).Replace('version = "1.2.4"', 'version = "1.2.5"').Replace('version = "2.3.4"', 'version = "2.3.5"')
                [IO.File]::WriteAllText($lockPath, $lock)
            }
            $global:LASTEXITCODE = 0
        }
        Assert-Rejected { & $updater 1.2.5 --project $fixture } 'unexpected dependency resolution'
    }
    Assert-Check ($receipt.Calls -ge 1) 'Lock mutation fixture did not reach Cargo.'
    Assert-Check ((& $hashes) -ceq $updatedHashes) 'Unexpected lock mutation did not restore both files.'
    Assert-Check (([IO.File]::ReadAllText($notePath) -ceq $note) -and ([IO.File]::ReadAllText($changelogPath) -ceq $changelog)) 'Version update modified release prose.'
    Write-Host 'PASS real Cargo version-only update, no-op and byte-for-byte rollback'
}
finally {
    foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, $environment[$name]) }
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
}
