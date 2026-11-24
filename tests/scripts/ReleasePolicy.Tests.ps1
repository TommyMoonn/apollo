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
foreach ($name in @('GITHUB_REF', 'GITHUB_REF_TYPE', 'GITHUB_REF_NAME', 'GITHUB_REPOSITORY', 'CARGO_NET_OFFLINE')) {
    $environment[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
    foreach ($name in @('GITHUB_REF', 'GITHUB_REF_TYPE', 'GITHUB_REF_NAME', 'GITHUB_REPOSITORY')) {
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

    # Use real Git commits and local tag objects. Substitute only the remote
    # transport: ls-remote reads the fixture itself, and gh returns fixture JSON.
    & {
        . (Join-Path $repo 'scripts/detect-release-candidate.ps1')
        $history = Join-Path $fixture 'candidate-history'
        $null = New-Item -ItemType Directory -Path $history, (Join-Path $history 'release-notes') -Force
        $identity = 'TommyMoonn/apollo'
        $state = @{ calls = 0; exact = $null; records = @(); fail = $false; malformed = $false; listFailure = $false; invalidList = $false; gitFailure = $false }
        function GitFixture([string[]]$Arguments) {
            (Invoke-RepositoryGit -ProjectRoot $history -Arguments $Arguments).StdOut.Trim()
        }
        function Read-CandidateGit([string]$ProjectRoot, [string[]]$Arguments) {
            if ($Arguments[0] -ceq 'ls-remote') {
                $remotePath = if ($state.gitFailure) { Join-Path $history "missing-remote.git" } else { $history }
                $Arguments = @($Arguments[0], $Arguments[1], $remotePath) + $Arguments[3..($Arguments.Count - 1)]
            }
            (Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments (@('--no-optional-locks', '--no-replace-objects') + $Arguments)).StdOut
        }
        function gh {
            $state.calls++
            $global:LASTEXITCODE = 0
            if ($args[0] -cne 'api' -or $args[1] -cne '--hostname' -or $args[2] -cne 'github.com') {
                throw 'Unexpected GitHub command in read-only fixture.'
            }
            if ($state.fail) { $global:LASTEXITCODE = 1; 'HTTP 401'; return }
            if ($args[3] -ceq 'graphql' -and $args -ccontains '--method' -and $args -ccontains 'POST' -and
                $args -ccontains 'tag=v0.10.0' -and @($args | Where-Object { $_ -clike 'query=query(*' }).Count -eq 1) {
                if ($state.malformed) { '{'; return }
                @{ data = @{ repository = @{ nameWithOwner = $identity; release = $state.exact } } } | ConvertTo-Json -Depth 8 -Compress
            }
            elseif ($args -ccontains 'GET' -and $args -ccontains '--paginate' -and $args -ccontains '--slurp' -and
                $args -ccontains "repos/$identity/releases?per_page=100") {
                if ($state.listFailure) { $global:LASTEXITCODE = 1; 'HTTP 403'; return }
                if ($state.invalidList) { '{}'; return }
                ConvertTo-Json -InputObject (,@($state.records)) -Depth 8 -Compress
            }
            else { throw 'Unexpected GitHub operation; no writes are permitted in this fixture.' }
        }
        function Write-FixtureVersion([string]$Version) {
            [IO.File]::WriteAllText((Join-Path $history 'Cargo.toml'), "[package]`nname = `"apollo`"`nversion = `"$Version`"`n")
            [IO.File]::WriteAllText((Join-Path $history 'Cargo.lock'), "version = 4`n`n[[package]]`nname = `"apollo`"`nversion = `"$Version`"`n")
        }
        function Commit-Fixture([string]$Message) {
            $null = GitFixture @('add', '-A')
            $null = GitFixture @('commit', '--allow-empty', '-m', $Message)
            GitFixture @('rev-parse', 'HEAD')
        }
        $null = GitFixture @('init', '--initial-branch=main')
        $null = GitFixture @('config', 'user.name', 'Release fixture')
        $null = GitFixture @('config', 'user.email', 'fixture@example.invalid')
        $null = GitFixture @('config', 'commit.gpgsign', 'false')
        $null = GitFixture @('config', 'tag.gpgsign', 'false')
        $null = GitFixture @('config', 'core.hooksPath', (Join-Path $fixture 'no-hooks'))
        $null = GitFixture @('remote', 'add', 'origin', "https://github.com/$identity.git")
        Write-FixtureVersion '0.9.0'
        $rootCommit = Commit-Fixture 'root'
        Assert-Rejected { Find-ReleaseCandidate $history $rootCommit $identity } 'root commit'
        $ordinary = Commit-Fixture 'ordinary change'
        $outputPath = Join-Path $fixture 'candidate-output.txt'
        $oldOutput = $env:GITHUB_OUTPUT
        try {
            $env:GITHUB_OUTPUT = $outputPath
            $result = Invoke-DetectReleaseCandidateCli @($ordinary, $identity, '--project', $history) | ConvertFrom-Json
            Assert-Check (-not $result.candidate -and $result.version -ceq '0.9.0' -and $result.sha -ceq $ordinary -and $result.reason) 'Ordinary commit was not skipped.'
            $lines = Get-Content -LiteralPath $outputPath
            Assert-Check (($lines -join "`n") -ceq "candidate=false`nversion=0.9.0`nsha=$ordinary`nreason=$($result.reason)") 'Noncandidate machine outputs differ from JSON.'
            Assert-Check ($state.calls -eq 0) 'Unchanged version contacted GitHub.'
        } finally { $env:GITHUB_OUTPUT = $oldOutput }
        Assert-Rejected { Find-ReleaseCandidate $history $rootCommit $identity } 'wrong checkout SHA'
        Assert-Rejected { Find-ReleaseCandidate $history 'abc' $identity } 'short SHA'
        Assert-Rejected { Find-ReleaseCandidate $history $ordinary 'owner/repo/extra' } 'invalid repository'
        Assert-Rejected { Find-ReleaseCandidate $history $ordinary 'other/apollo' } 'wrong repository'
        Write-FixtureVersion '0.10.0'
        Assert-Rejected { Find-ReleaseCandidate $history $ordinary $identity } 'dirty checkout'
        $badMetadata = Commit-Fixture 'version without release metadata'
        Assert-Rejected { Find-ReleaseCandidate $history $badMetadata $identity } 'missing candidate metadata'
        $null = GitFixture @('reset', '--hard', $ordinary)
        $null = GitFixture @('switch', '-c', 'release')
        Write-FixtureVersion '0.10.0'
        [IO.File]::WriteAllText((Join-Path $history 'CHANGELOG.md'), "## [0.10.0] - 2028-02-29`n`n[0.10.0]: https://github.com/TommyMoonn/apollo/compare/v0.9.0...v0.10.0`n")
        [IO.File]::WriteAllText((Join-Path $history 'release-notes/v0.10.0.md'), $note.Replace('v1.2.3', 'v0.10.0'))
        $null = Commit-Fixture 'release branch'
        $null = GitFixture @('switch', 'main')
        $null = Commit-Fixture 'main branch change'
        $null = GitFixture @('merge', '--no-ff', '--no-edit', 'release')
        $sha = GitFixture @('rev-parse', 'HEAD')
        $detect = { Find-ReleaseCandidate $history $sha $identity }
        $before = GitFixture @('show-ref')
        $oldOutput = $env:GITHUB_OUTPUT
        try {
            $env:GITHUB_OUTPUT = $outputPath
            [IO.File]::WriteAllText($outputPath, '')
            $candidate = Invoke-DetectReleaseCandidateCli @($sha, $identity, '--project', $history) | ConvertFrom-Json
            Assert-Check ($candidate.candidate -and $candidate.version -ceq '0.10.0' -and $candidate.sha -ceq $sha) 'Numeric increase on merge first parent was not detected.'
            Assert-Check (((Get-Content -LiteralPath $outputPath) -join "`n") -ceq "candidate=true`nversion=0.10.0`nsha=$sha`nreason=") 'Candidate machine outputs differ from JSON.'
        } finally { $env:GITHUB_OUTPUT = $oldOutput }
        Assert-Check ((GitFixture @('show-ref')) -ceq $before) 'Detection changed Git refs.'
        Assert-Check ((GitFixture @('status', '--porcelain')).Length -eq 0) 'Detection changed the checkout.'
        $null = GitFixture @('tag', 'v0.10.0', $ordinary)
        Assert-Rejected $detect 'wrong remote tag target'
        $null = GitFixture @('tag', '-d', 'v0.10.0')
        $null = GitFixture @('tag', 'v0.10.0', $sha)
        Assert-Check ((& $detect).candidate) 'Matching lightweight tag was rejected.'
        $null = GitFixture @('tag', '-d', 'v0.10.0')
        $null = GitFixture @('tag', '-a', 'v0.10.0', '-m', 'fixture tag', $sha)
        $state.exact = @{ databaseId = 42; isDraft = $true }
        $record = @{ id = 42; draft = $true; tag_name = 'untagged-aabb'; name = 'Apollo v0.10.0'; target_commitish = $sha }
        $state.records = @($record)
        Assert-Check ((& $detect).candidate) 'Matching annotated tag and pending draft were rejected.'
        $state.records = @($record, @{ id = 43; draft = $true; tag_name = 'untagged-bbcc'; name = 'Apollo v0.10.0'; target_commitish = $sha })
        Assert-Rejected $detect 'ambiguous draft'
        $state.records = @($record)
        $record.target_commitish = $ordinary
        Assert-Rejected $detect 'conflicting draft commit'
        $record.target_commitish = $sha
        $state.exact = $null
        Assert-Rejected $detect 'unresolved draft'
        $state.exact = @{ databaseId = 42; isDraft = $false }
        $record.draft = $false
        $record.tag_name = 'v0.10.0'
        Assert-Check ((& $detect).candidate) 'Matching published release replay was rejected.'
        $null = GitFixture @('tag', '-d', 'v0.10.0')
        Assert-Rejected $detect 'release without matching tag'
        $state.exact = $null
        $state.records = @()
        $state.fail = $true
        Assert-Rejected $detect 'authentication failure'
        $state.fail = $false
        $state.malformed = $true
        Assert-Rejected $detect 'malformed GitHub JSON'
        $state.malformed = $false
        $state.listFailure = $true
        Assert-Rejected $detect 'failed release enumeration'
        $state.listFailure = $false
        $state.invalidList = $true
        Assert-Rejected $detect 'malformed release list'
        $state.invalidList = $false
        $state.gitFailure = $true
        Assert-Rejected $detect 'failed Git tag lookup'
        $state.gitFailure = $false
        Assert-Check ((& $detect).candidate) 'Confirmed missing tag and release were rejected.'
        Write-FixtureVersion '0.9.0'
        $down = Commit-Fixture 'downgrade'
        Assert-Rejected { Find-ReleaseCandidate $history $down $identity } 'downgrade'
        Write-FixtureVersion '0.11.0-rc.1'
        $invalid = Commit-Fixture 'prerelease'
        Assert-Rejected { Find-ReleaseCandidate $history $invalid $identity } 'prerelease'
        Write-FixtureVersion '0.11.0'
        $invalidParent = Commit-Fixture 'stable after prerelease'
        Assert-Rejected { Find-ReleaseCandidate $history $invalidParent $identity } 'invalid parent version'
        Write-Host 'PASS read-only candidate transitions, first-parent identity, remote conflicts and machine outputs'
    }

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
    # Use a real built Apollo executable for PE/resource checks. The other fixture
    # bytes only exercise artifact integrity; they are not installer/media acceptance.
    . (Join-Path $repo 'scripts/stage-windows-release.ps1')
    $mediaManifestPath = Join-Path $repo 'distribution/media-tools.json'
    $media = Read-MediaToolsManifest $mediaManifestPath
    $badCache = Join-Path $fixture 'corrupt-cache'
    [void][IO.Directory]::CreateDirectory($badCache)
    [IO.File]::WriteAllText((Join-Path $badCache $media.archive.filename), 'corrupt archive')
    $mediaDestination = Join-Path $fixture 'media-output'
    Assert-Rejected { Publish-MediaTools -ManifestPath $mediaManifestPath -CachePath $badCache -OutputPath $mediaDestination -Offline } 'corrupt cached media archive'
    Assert-Check (-not (Test-Path -LiteralPath $mediaDestination)) 'Corrupt cache published a payload.'
    Write-Host 'PASS corrupt media cache cannot publish a payload'

    # Exercise the smoke policy with disposable files, not a real installation or registry.
    . (Join-Path $repo 'tests/scripts/WindowsRelease.Smoke.ps1')
    $smokeRoot = Join-Path $fixture 'smoke-policy'
    [void][IO.Directory]::CreateDirectory($smokeRoot)
    $fakeInstaller = Join-Path $smokeRoot 'Apollo-Setup-x64.exe'
    $fakeSums = Join-Path $smokeRoot 'SHA256SUMS.txt'
    [IO.File]::WriteAllText($fakeInstaller, 'installer fixture')
    $sum = (Get-ReleaseFileRecord $fakeInstaller).sha256
    [IO.File]::WriteAllText($fakeSums, "$sum  Apollo-Setup-x64.exe`n")
    $assets = @(@{ name = 'Apollo-Setup-x64.exe'; state = 'uploaded'; size = (Get-Item $fakeInstaller).Length },
        @{ name = 'SHA256SUMS.txt'; state = 'uploaded'; size = (Get-Item $fakeSums).Length })
    $releases = @(@{ tag_name = 'v0.6.0'; draft = $false; prerelease = $false; assets = $assets },
        @{ tag_name = 'v0.7.0'; draft = $false; prerelease = $false; assets = $assets },
        @{ tag_name = 'v0.8.0'; draft = $true; prerelease = $false; assets = $assets })
    $selected = Get-SmokePredecessor $releases '0.7.0'
    Assert-Check ($selected.tag_name -ceq 'v0.7.0') 'Current published version must exercise reinstall.'
    Assert-Check ((Get-SmokePredecessor $releases '0.7.0' -UpgradeOnly).tag_name -ceq 'v0.6.0') 'Upgrade must select an older published version.'
    Assert-Check ((Get-SmokePredecessor $releases '0.8.0' -UpgradeOnly).tag_name -ceq 'v0.7.0') 'Draft must not be a predecessor.'
    Assert-Rejected { Get-SmokePredecessor $releases '0.6.0' -UpgradeOnly } 'missing predecessor'
    $null = Assert-SmokePredecessorChecksum $smokeRoot $selected
    [IO.File]::WriteAllText($fakeInstaller, 'installer fixturE')
    Assert-Rejected { Assert-SmokePredecessorChecksum $smokeRoot $selected } 'corrupt predecessor'
    $releases[1].assets = @()
    Assert-Rejected { Get-SmokePredecessor $releases '0.7.0' } 'missing checksum on selected release must not silently fall back'

    $payloadRoot = Join-Path $smokeRoot 'payload'
    [void][IO.Directory]::CreateDirectory($payloadRoot)
    $payloadPath = Join-Path $payloadRoot 'Apollo.exe'
    [IO.File]::WriteAllText($payloadPath, 'payload fixture')
    $payload = [ordered]@{ 'Apollo.exe' = Get-ReleaseFileRecord $payloadPath }
    Assert-SmokePayload $payloadRoot $payload
    $sentinels = New-SmokeSentinels $payloadRoot
    Assert-SmokePayload $payloadRoot $payload -ExtraFiles @($sentinels.Keys)
    Assert-SmokeSentinels $payloadRoot $sentinels
    [IO.File]::WriteAllText((Join-Path $payloadRoot 'keep.txt'), 'changed user content')
    Assert-Rejected { Assert-SmokeSentinels $payloadRoot $sentinels } 'modified user sentinel'
    [IO.File]::WriteAllText($payloadPath, 'wrong payload')
    Assert-Rejected { Assert-SmokePayload $payloadRoot $payload -ExtraFiles @($sentinels.Keys) } 'wrong installed bytes'
    Assert-Rejected { Assert-SmokeCleanState @() @() $payloadRoot } 'existing installation directory'
    Assert-Check (Test-Path -LiteralPath $payloadPath) 'Clean-state rejection removed pre-existing files.'
    if ($IsWindows) {
        $hostExecutable = (Get-Process -Id $PID).Path
        Invoke-SmokeProcess $hostExecutable '-NoProfile -NonInteractive -Command "exit 0"' -Scratch $smokeRoot
        Assert-Rejected { Invoke-SmokeProcess $hostExecutable '-NoProfile -NonInteractive -Command "exit 17"' -Scratch $smokeRoot } 'failed installer process'
        $timedOut = $false
        try {
            Invoke-SmokeProcess $hostExecutable '-NoProfile -NonInteractive -Command "Start-Sleep -Seconds 60"' -Scratch $smokeRoot -TimeoutMilliseconds 500
        } catch [TimeoutException] { $timedOut = $true }
        Assert-Check $timedOut 'Blocked child did not trigger the bounded timeout.'
    }
    Write-Host 'PASS predecessor policy, checksum, payload, sentinel and clean-state rejection; Windows process checks when available'

    $builtExecutable = Join-Path $repo 'target/x86_64-pc-windows-msvc/release/apollo.exe'
    if (-not $IsWindows -or -not (Test-Path -LiteralPath $builtExecutable -PathType Leaf)) {
        Write-Host 'SKIP artifact fixtures: run just build-release on Windows first, then rerun this check.'
    } else {
        Assert-ApolloResourceVersion -Path $builtExecutable -Version $current.Version
        $sourceRoot = Join-Path $fixture 'artifact-source'
        $payloadRoot = Join-Path $fixture 'artifact-payload'
        $artifactRoot = Join-Path $fixture 'artifact-output'
        foreach ($dir in @($sourceRoot, $payloadRoot, $artifactRoot, (Join-Path $sourceRoot 'distribution'))) {
            [void][IO.Directory]::CreateDirectory($dir)
        }
        foreach ($name in @('Cargo.toml','Cargo.lock','rust-toolchain.toml','LICENSE','THIRD_PARTY_NOTICES.md','distribution/media-tools.json')) {
            Copy-Item -LiteralPath (Join-Path $repo $name) -Destination (Join-Path $sourceRoot $name)
        }
        $null = Invoke-RepositoryGit -ProjectRoot $sourceRoot -Arguments @('init','--quiet')
        $null = Invoke-RepositoryGit -ProjectRoot $sourceRoot -Arguments @('add','--all')
        $null = Invoke-RepositoryGit -ProjectRoot $sourceRoot -Arguments @('-c','user.name=Release fixture','-c','user.email=fixture@example.invalid','-c','commit.gpgsign=false','commit','--quiet','-m','fixture')
        $sha = (Invoke-RepositoryGit -ProjectRoot $sourceRoot -Arguments @('rev-parse','HEAD')).StdOut.Trim()
        foreach ($name in (Get-MediaToolPayloadPaths $media)) {
            $path = Join-Path $payloadRoot $name
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))
            if ($name -ceq 'licenses/ffmpeg/media-tools.json') { Copy-Item -LiteralPath $mediaManifestPath -Destination $path }
            else { [IO.File]::WriteAllText($path, "fixture bytes for $name") }
        }
        $sources = Get-WindowsPayloadSources -ProjectRoot $sourceRoot -ExecutablePath $builtExecutable -MediaToolsPath $payloadRoot -MediaManifest $media
        $zipPath = Join-Path $artifactRoot 'Apollo-windows-x64.zip'
        $sumPath = Join-Path $artifactRoot 'SHA256SUMS.txt'
        $provenancePath = Join-Path $artifactRoot 'release-provenance.json'
        $installerPath = Join-Path $artifactRoot 'Apollo-Setup-x64.exe'
        [IO.File]::WriteAllText($installerPath, 'fixture installer bytes, not executable')
        New-WindowsReleaseZip -ZipPath $zipPath -PayloadSources $sources
        $pristineZip = Join-Path $fixture 'pristine.zip'
        Copy-Item -LiteralPath $zipPath -Destination $pristineZip
        $payloadRecords = [ordered]@{}
        foreach ($name in $sources.Keys) { $payloadRecords[$name] = Get-ReleaseFileRecord $sources[$name] }
        $provenance = [ordered]@{
            schemaVersion = 1; candidateSha = $sha; version = $current.Version
            target = 'x86_64-pc-windows-msvc'; rustVersion = Get-ReleaseRustPin $sourceRoot; nsisVersion = '3.11'
            mediaArchive = [ordered]@{ filename = $media.archive.filename; url = $media.archive.url; sha256 = $media.archive.sha256; buildIdentity = $media.buildIdentity }
            assets = [ordered]@{}; payload = $payloadRecords
        }
        function Write-FixtureProvenance {
            foreach ($name in @('Apollo-Setup-x64.exe','Apollo-windows-x64.zip')) { $provenance.assets[$name] = Get-ReleaseFileRecord (Join-Path $artifactRoot $name) }
            $lines = foreach ($name in @('Apollo-Setup-x64.exe','Apollo-windows-x64.zip')) { "$($provenance.assets[$name].sha256)  $name" }
            [IO.File]::WriteAllText($sumPath, (($lines -join "`n") + "`n"))
            $provenance.assets['SHA256SUMS.txt'] = Get-ReleaseFileRecord $sumPath
            [IO.File]::WriteAllText($provenancePath, ($provenance | ConvertTo-Json -Depth 8))
        }
        Write-FixtureProvenance
        $verify = { Assert-WindowsReleaseArtifacts -ProjectRoot $sourceRoot -ArtifactsDirectory $artifactRoot -ExpectedSha $sha -ExpectedVersion $current.Version }
        & $verify
        $originalJson = [IO.File]::ReadAllText($provenancePath)
        foreach ($field in @('candidateSha','version','target','schemaVersion','rustVersion','nsisVersion')) {
            $original = $provenance[$field]
            $provenance[$field] = 'invalid'
            [IO.File]::WriteAllText($provenancePath, ($provenance | ConvertTo-Json -Depth 8))
            Assert-Rejected $verify "wrong provenance $field"
            $provenance[$field] = $original
        }
        foreach ($json in @('{', ('{"schemaVersion":1,' + $originalJson.TrimStart().Substring(1)))) {
            [IO.File]::WriteAllText($provenancePath, $json)
            Assert-Rejected $verify 'malformed or duplicate-key provenance'
        }
        [IO.File]::WriteAllText($provenancePath, $originalJson)
        $provenance.candidateSha = $null
        [IO.File]::WriteAllText($provenancePath, ($provenance | ConvertTo-Json -Depth 8))
        Assert-Rejected $verify 'local provenance presented as a candidate'
        Assert-WindowsReleaseArtifacts -ProjectRoot $sourceRoot -ArtifactsDirectory $artifactRoot -ExpectedVersion $current.Version -Local
        $provenance.candidateSha = $sha
        [IO.File]::WriteAllText($provenancePath, $originalJson)
        $extra = Join-Path $artifactRoot 'unrelated.txt'
        [IO.File]::WriteAllText($extra, 'keep')
        Assert-Rejected $verify 'extra artifact'
        Assert-Rejected { Assert-SafeStagingDirectory -ProjectRoot $sourceRoot -ExecutablePath $builtExecutable -OutputPath $artifactRoot } 'unrelated staging content'
        Assert-Check ([IO.File]::ReadAllText($extra) -ceq 'keep') 'Unrelated file changed.'
        Remove-Item -LiteralPath $extra
        Move-Item -LiteralPath $installerPath -Destination (Join-Path $fixture 'saved-installer')
        Assert-Rejected $verify 'missing artifact'
        Move-Item -LiteralPath (Join-Path $fixture 'saved-installer') -Destination $installerPath
        $originalSums = [IO.File]::ReadAllText($sumPath)
        foreach ($sums in @($originalSums.Replace('  Apollo-', ' Apollo-'), ($originalSums + $originalSums.Split("`n")[0] + "`n"))) {
            [IO.File]::WriteAllText($sumPath, $sums)
            $provenance.assets['SHA256SUMS.txt'] = Get-ReleaseFileRecord $sumPath
            [IO.File]::WriteAllText($provenancePath, ($provenance | ConvertTo-Json -Depth 8))
            Assert-Rejected $verify 'malformed or duplicate checksum'
        }
        foreach ($case in @('duplicate','case collision','traversal','rooted','missing license','corrupt payload')) {
            Copy-Item -LiteralPath $pristineZip -Destination $zipPath -Force
            $archive = [IO.Compression.ZipFile]::Open($zipPath, [IO.Compression.ZipArchiveMode]::Update)
            try {
                switch ($case) {
                    'duplicate' { $null = $archive.CreateEntry('Apollo.exe') }
                    'case collision' { $null = $archive.CreateEntry('apollo.exe') }
                    'traversal' { $null = $archive.CreateEntry('../outside.txt') }
                    'rooted' { $null = $archive.CreateEntry('/outside.txt') }
                    'missing license' { $archive.GetEntry('licenses/ffmpeg/LICENSE').Delete() }
                    'corrupt payload' {
                        $entry = $archive.GetEntry('ffmpeg.exe')
                        $stream = $entry.Open()
                        try { $stream.WriteByte(0) } finally { $stream.Dispose() }
                    }
                }
            } finally { $archive.Dispose() }
            # Update the outer hashes, not payload records, to reach the ZIP checks.
            Write-FixtureProvenance
            Assert-Rejected $verify $case
        }
        Copy-Item -LiteralPath $pristineZip -Destination $zipPath -Force
        Write-FixtureProvenance
        & $verify
        $otherVersion = if ($current.Version -ceq '0.0.0') { '0.0.1' } else { '0.0.0' }
        Assert-Rejected { Assert-ApolloResourceVersion -Path $builtExecutable -Version $otherVersion } 'incorrect resource version'
        $wrongExe = Join-Path $fixture 'apollo.exe'
        Copy-Item -LiteralPath $builtExecutable -Destination $wrongExe
        $bytes = [IO.File]::ReadAllBytes($wrongExe)
        $offset = [BitConverter]::ToInt32($bytes, 0x3c) + 4
        $bytes[$offset] = 0x4c; $bytes[$offset + 1] = 0x01
        [IO.File]::WriteAllBytes($wrongExe, $bytes)
        Assert-Rejected { Assert-ApolloResourceVersion -Path $wrongExe -Version $current.Version } 'incorrect PE architecture'
        Write-Host 'PASS independent artifact identity, checksum coverage, ZIP payload and PE checks'

        $compiler = $null
        try { $compiler = Resolve-NsisCompilerPath -ProjectRoot $repo -Value $null } catch { }
        if ($null -eq $compiler) { Write-Host 'SKIP staging failure fixture: install NSIS 3.11 first.' }
        else {
            $compilerVersion = & $compiler /VERSION
            Assert-Check ($LASTEXITCODE -eq 0 -and "$compilerVersion".Trim() -ceq 'v3.11') 'Staging failure fixture requires NSIS v3.11.'
            $before = @((Get-WindowsArtifactNames) | ForEach-Object { (Get-ReleaseFileRecord (Join-Path $artifactRoot $_)).sha256 }) -join ':'
            Assert-Rejected { Invoke-StageWindowsReleaseCli -Arguments @('--project',$repo,'--executable',$builtExecutable,'--output',$artifactRoot,'--makensis',$compiler,'--media-cache',$badCache,'--offline') } 'failed staging with corrupt cache'
            $after = @((Get-WindowsArtifactNames) | ForEach-Object { (Get-ReleaseFileRecord (Join-Path $artifactRoot $_)).sha256 }) -join ':'
            Assert-Check ($before -ceq $after) 'Failed staging modified the previous set.'
            & $verify
            Write-Host 'PASS failed staging preserved previous output'
        }
    }

}
finally {
    foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, $environment[$name]) }
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
}
