#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
. (Join-Path $PSScriptRoot 'Test.Common.ps1')
. (Join-Path $projectRoot 'scripts/Repository.Common.ps1')

$zipProjectScript = Join-Path $projectRoot 'scripts/zip-project.ps1'
$reviewScript = Join-Path $projectRoot 'scripts/review-changes.ps1'
$packageScript = Join-Path $projectRoot 'scripts/package-changes.ps1'
$applyScript = Join-Path $projectRoot 'scripts/apply-changes.ps1'
$restoreScript = Join-Path $projectRoot 'scripts/restore-changes.ps1'
$cleanScript = Join-Path $projectRoot 'scripts/clean-generated.ps1'


Invoke-TestCase 'canonical tooling commands expose help without side effects' {
    foreach ($scriptPath in @($zipProjectScript, $reviewScript, $packageScript, $applyScript, $restoreScript, $cleanScript)) {
        $output = Invoke-ScriptCapture -ScriptPath $scriptPath -Arguments @('--help')
        Assert-Contains $output 'Usage:' "Help output missing for $scriptPath."
    }
}

Invoke-TestCase 'repository path guard rejects traversal and .git' {
    $repo = New-TestDirectory
    try {
        Assert-Throws { Resolve-RepositoryRelativePath -ProjectRoot $repo -RelativePath '../outside.txt' } '*traversal*'
        Assert-Throws { Resolve-RepositoryRelativePath -ProjectRoot $repo -RelativePath '.git/config' } '*.git*'
        Assert-Throws { Resolve-RepositoryRelativePath -ProjectRoot $repo -RelativePath 'nested/.git/config' } '*.git*'
    }
    finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

function Copy-SnapshotFixtureScripts {
    param([string]$Repository)
    New-Item -ItemType Directory -Path (Join-Path $Repository 'scripts') -Force | Out-Null
    foreach ($name in @('zip-project.ps1', 'Cli.Common.ps1', 'Repository.Common.ps1')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "scripts/$name") -Destination (Join-Path $Repository "scripts/$name")
    }
}

function New-SnapshotFixtureFile {
    param([string]$Repository, [string]$RelativePath)
    $path = Join-Path $Repository $RelativePath
    New-Item -ItemType Directory -Path (Split-Path $path -Parent) -Force | Out-Null
    Set-Content -LiteralPath $path -Value 'fixture' -Encoding utf8
}

function Assert-ExportManifest {
    param([string]$Archive, [string]$Mode, $Commit, $Dirty, [string]$Baseline, [bool]$Planning, [bool]$Project)
    $entries = @(Get-ZipEntryNames -Path $Archive)
    Assert-Equal 1 @($entries | Where-Object { $_ -ceq 'EXPORT_MANIFEST.json' }).Count 'Archive must contain one root manifest.'
    $manifest = Get-ZipEntryText -Path $Archive -EntryName 'EXPORT_MANIFEST.json' | ConvertFrom-Json
    Assert-Equal 1 $manifest.schemaVersion 'Schema version mismatch.'
    Assert-Equal $Mode $manifest.exportMode 'Export mode mismatch.'
    Assert-Equal $Commit $manifest.sourceCommit 'Source commit mismatch.'
    if ($null -eq $Dirty) { Assert-True ($null -eq $manifest.workingTreeDirty) 'Non-Git dirty state must be null.' }
    else {
        Assert-True ($manifest.workingTreeDirty -is [bool]) 'Git dirty state must be a boolean.'
        Assert-Equal $Dirty $manifest.workingTreeDirty 'Dirty state mismatch.'
    }
    Assert-Equal $Baseline $manifest.trackedSourceBaseline 'Baseline mismatch.'
    Assert-True ($manifest.includedLocalOnly.planning -is [bool] -and $manifest.includedLocalOnly.project -is [bool]) 'Local inclusion flags must be booleans.'
    Assert-Equal $Planning $manifest.includedLocalOnly.planning 'Planning inclusion mismatch.'
    Assert-Equal $Project $manifest.includedLocalOnly.project 'Project inclusion mismatch.'
    # ConvertFrom-Json may parse ISO timestamps as DateTime in newer PowerShell versions.
    $raw = Get-ZipEntryText -Path $Archive -EntryName 'EXPORT_MANIFEST.json'
    Assert-True ($raw -match '"createdAt"\s*:\s*"[^"\r\n]+Z"') 'Manifest creation time must be UTC ISO-8601.'
    $createdAt = [regex]::Match($raw, '"createdAt"\s*:\s*"([^"]+)"').Groups[1].Value
    [void][DateTimeOffset]::Parse($createdAt, [System.Globalization.CultureInfo]::InvariantCulture)
    Assert-Equal 7 @($manifest.PSObject.Properties).Count 'Unexpected provenance fields.'
}

Invoke-TestCase 'zip-project uses real exclusions and preserves required source and local context' {
    $parent = New-TestDirectory -Name 'apollo-zip-parent'
    $repo = Join-Path $parent 'Apollo Sample'
    try {
        Copy-SnapshotFixtureScripts -Repository $repo
        Copy-Item -LiteralPath (Join-Path $projectRoot '.zipignore') -Destination $repo
        $included = @('README.md', 'Cargo.toml', 'Cargo.lock', 'src/main.rs', 'assets/icon.png', 'tests/sample.rs',
            '.plans/plan.md', '.planning/legacy.md', '.project/review.md', '.agents/skills/example/SKILL.md', 'docs/space name.md', 'tests/fixtures/target/fixture.txt', 'tests/fixtures/artifacts/fixture.txt')
        $excluded = @('.git/config', 'target/build.txt', 'artifacts/result.txt', '.dev/chatgpt/bundles/bundle.txt',
            '.scratch/cache.txt', '.agent-work/prepare.ps1', 'nested/.env.local', '.env', 'private.pem',
            'nested/run.log', 'test-results/report.txt', 'coverage/report.txt', 'apollo-cargo-vendor-offline-bundle.part-000.zip', 'temporary.tmp', 'Desktop.ini')
        foreach ($path in ($included + $excluded)) { New-SnapshotFixtureFile -Repository $repo -RelativePath $path }

        Invoke-ScriptCapture -ScriptPath (Join-Path $repo 'scripts/zip-project.ps1') -Arguments @('--mode', 'workspace') | Out-Null
        $archives = @(Get-ChildItem -LiteralPath $parent -File -Filter 'apollo-workspace(*).zip')
        Assert-Equal 1 $archives.Count 'Expected one project snapshot.'
        Assert-True ($archives[0].Name -match '^apollo-workspace\(\d{10}\)\.zip$') 'Snapshot filename should use mode and yyMMddHHmm timestamp.'
        $entries = @(Get-ZipEntryNames -Path $archives[0].FullName)
        foreach ($path in ($included + @('scripts/zip-project.ps1', '.zipignore'))) {
            Assert-True ($entries -ccontains $path) "Required project-relative entry missing: $path"
        }
        foreach ($path in $excluded) { Assert-False ($entries -ccontains $path) "Excluded entry leaked: $path" }
        Assert-ExportManifest -Archive $archives[0].FullName -Mode workspace -Commit $null -Dirty $null -Baseline 'working-tree' -Planning $true -Project $true
    }
    finally { Remove-Item -LiteralPath $parent -Recurse -Force }
}

Invoke-TestCase 'zip-project respects anchored, basename, directory and wildcard patterns' {
    $parent = New-TestDirectory -Name 'apollo-zip-patterns'
    $repo = Join-Path $parent 'Apollo Patterns'
    try {
        Copy-SnapshotFixtureScripts -Repository $repo
        @('# comment', '', '/root-only', 'cache/', 'build/**/report?.txt', '*.log') | Set-Content -LiteralPath (Join-Path $repo '.zipignore') -Encoding utf8
        $included = @('nested/root-only', 'cache', 'build/report12.txt', 'build/a/other.txt')
        $excluded = @('root-only', 'nested/cache/file.txt', 'build/report1.txt', 'build/a/b/report2.txt', 'nested/run.log')
        foreach ($path in ($included + $excluded)) { New-SnapshotFixtureFile -Repository $repo -RelativePath $path }
        Invoke-ScriptCapture -ScriptPath (Join-Path $repo 'scripts/zip-project.ps1') -Arguments @('--mode', 'workspace') | Out-Null
        $archive = Get-ChildItem -LiteralPath $parent -File -Filter '*.zip' | Select-Object -First 1
        $entries = @(Get-ZipEntryNames -Path $archive.FullName)
        foreach ($path in $included) { Assert-True ($entries -ccontains $path) "Expected entry missing: $path" }
        foreach ($path in $excluded) { Assert-False ($entries -ccontains $path) "Excluded entry leaked: $path" }
    }
    finally { Remove-Item -LiteralPath $parent -Recurse -Force }
}

Invoke-TestCase 'zip-project rejects invalid options and ignore negation without producing an archive' {
    $parent = New-TestDirectory -Name 'apollo-zip-invalid'
    $repo = Join-Path $parent 'Apollo Invalid'
    try {
        Copy-SnapshotFixtureScripts -Repository $repo
        $scriptCopy = Join-Path $repo 'scripts/zip-project.ps1'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy -Arguments @('--unknown') } 'Unknown option*'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy -Arguments @('--help', '-h') } '*provided more than once*'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy } '*Specify --mode*'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy -Arguments @('--mode', 'invalid') } '*repo, workspace*'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy -Arguments @('--mode', 'workspace') } '*.zipignore*'
        '!secret.txt' | Set-Content -LiteralPath (Join-Path $repo '.zipignore')
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy -Arguments @('--mode', 'workspace') } '*negation is not supported*'
        Assert-Equal 0 @(Get-ChildItem -LiteralPath $parent -File).Count 'Invalid requests must not leave output.'
    }
    finally { Remove-Item -LiteralPath $parent -Recurse -Force }
}

Invoke-TestCase 'zip-project removes partial output when compression cannot read a file' {
    $parent = New-TestDirectory -Name 'apollo-zip-failure'
    $repo = Join-Path $parent 'Apollo Failure'
    try {
        Copy-SnapshotFixtureScripts -Repository $repo
        '# no exclusions needed' | Set-Content -LiteralPath (Join-Path $repo '.zipignore')
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'locked.txt'
        $locked = [System.IO.File]::Open((Join-Path $repo 'locked.txt'), [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
        try {
            Assert-Throws { Invoke-ScriptCapture -ScriptPath (Join-Path $repo 'scripts/zip-project.ps1') -Arguments @('--mode', 'workspace') } '*locked.txt*'
        }
        finally { $locked.Dispose() }
        Assert-Equal 0 @(Get-ChildItem -LiteralPath $parent -File -Force).Count 'Failure must leave neither final nor temporary archives.'
    }
    finally { Remove-Item -LiteralPath $parent -Recurse -Force }
}

Invoke-TestCase 'zip-project help creates no archive' {
    $parent = New-TestDirectory -Name 'apollo-zip-help-parent'
    $repo = Join-Path $parent 'apollo-help'
    New-Item -ItemType Directory -Path (Join-Path $repo 'scripts') -Force | Out-Null
    try {
        Copy-Item -LiteralPath $zipProjectScript -Destination (Join-Path $repo 'scripts/zip-project.ps1')
        Copy-Item -LiteralPath (Join-Path $projectRoot 'scripts/Cli.Common.ps1') -Destination (Join-Path $repo 'scripts/Cli.Common.ps1')
        Copy-Item -LiteralPath (Join-Path $projectRoot 'scripts/Repository.Common.ps1') -Destination (Join-Path $repo 'scripts/Repository.Common.ps1')
        Invoke-ScriptCapture -ScriptPath (Join-Path $repo 'scripts/zip-project.ps1') -Arguments @('--help') | Out-Null
        Assert-Equal 0 @(Get-ChildItem -LiteralPath $parent -File -Filter '*.zip').Count 'Help should not create an archive.'
    }
    finally { Remove-Item -LiteralPath $parent -Recurse -Force }
}

Invoke-TestCase 'repo and workspace exports have distinct content and accurate provenance' {
    $parent = New-TestDirectory -Name 'apollo export'
    $repo = Join-Path $parent 'project with spaces'
    New-Item -ItemType Directory -Path $repo | Out-Null
    try {
        & git -C $repo init --quiet
        & git -C $repo config user.email 'apollo-tests@example.invalid'
        & git -C $repo config user.name 'Apollo Tests'
        Copy-Item -LiteralPath (Join-Path $projectRoot '.gitignore') -Destination $repo
        Copy-Item -LiteralPath (Join-Path $projectRoot '.zipignore') -Destination $repo
        Set-Content -LiteralPath (Join-Path $repo 'tracked.txt') -Value 'committed' -NoNewline
        & git -C $repo add .
        & git -C $repo commit --quiet -m 'test: committed source'
        if ($LASTEXITCODE -ne 0) { throw 'Fixture commit failed.' }
        $head = (& git -C $repo rev-parse HEAD).Trim()
        Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '-p', $repo) | Out-Null
        $cleanArchives = @(Get-ChildItem -LiteralPath $parent -File -Filter 'apollo-repo(*).zip')
        Assert-Equal 1 $cleanArchives.Count 'Expected one default repo archive.'
        Assert-True ($cleanArchives[0].Name -match '^apollo-repo\(\d{10}\)\.zip$') 'Repo filename must use the mode and timestamp.'
        $clean = $cleanArchives[0].FullName
        Assert-ExportManifest -Archive $clean -Mode repo -Commit $head -Dirty $false -Baseline 'git-commit' -Planning $false -Project $false
        Set-Content -LiteralPath (Join-Path $repo 'tracked.txt') -Value 'working' -NoNewline
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'untracked.txt'
        New-SnapshotFixtureFile -Repository $repo -RelativePath '.plans/plan.md'
        New-SnapshotFixtureFile -Repository $repo -RelativePath '.project/record.md'
        $committed = Join-Path $parent 'dirty repo.zip'
        $workspace = Join-Path $parent 'dirty workspace.zip'
        Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '--project', $repo, '--output', $committed) | Out-Null
        Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'workspace', '--project', $repo, '--output', $workspace) | Out-Null
        $expected = @(& git -C $repo ls-tree -r --name-only HEAD) + @('EXPORT_MANIFEST.json')
        $actual = @(Get-ZipEntryNames -Path $committed | Where-Object { -not $_.EndsWith('/') })
        Assert-Equal @($expected | Sort-Object) @($actual | Sort-Object) 'Repo contents must equal committed files plus manifest.'
        Assert-Equal 'committed' (Get-ZipEntryText -Path $committed -EntryName 'tracked.txt') 'Repo must contain committed bytes.'
        Assert-Equal 'working' (Get-ZipEntryText -Path $workspace -EntryName 'tracked.txt') 'Workspace must contain current bytes.'
        $workspaceEntries = @(Get-ZipEntryNames -Path $workspace)
        foreach ($name in @('untracked.txt', '.plans/plan.md', '.project/record.md')) { Assert-True ($workspaceEntries -ccontains $name) "Workspace omitted $name." }
        Assert-ExportManifest -Archive $committed -Mode repo -Commit $head -Dirty $true -Baseline 'git-commit' -Planning $false -Project $false
        Assert-ExportManifest -Archive $workspace -Mode workspace -Commit $head -Dirty $true -Baseline 'working-tree-over-commit' -Planning $true -Project $true
        $original = (Get-FileHash -LiteralPath $workspace).Hash
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'workspace', '-p', $repo, '-o', $workspace) } '*already exists*'
        Assert-Equal $original (Get-FileHash -LiteralPath $workspace).Hash 'Existing output must not be overwritten.'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '-p', $repo, '-o', (Join-Path $repo 'inside.zip')) } '*outside*'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '-p', $repo, '-o', (Join-Path $parent 'bad.txt')) } '*end in .zip*'
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'EXPORT_MANIFEST.json'
        $bad = Join-Path $parent 'collision.zip'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'workspace', '-p', $repo, '-o', $bad) } '*reserved manifest*'
        & git -C $repo add EXPORT_MANIFEST.json
        & git -C $repo commit --quiet -m 'test: reserved manifest'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '-p', $repo, '-o', $bad) } '*reserved manifest*'
        & git -C $repo rm --quiet EXPORT_MANIFEST.json
        & git -C $repo add -f .plans/plan.md
        & git -C $repo commit --quiet -m 'test: forbidden local record'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '-p', $repo, '-o', $bad) } '*committed local-only*'
        Assert-PathMissing -Path $bad -Message 'Rejected exports must not be published.'
        Assert-Equal 0 @(Get-ChildItem -LiteralPath $parent -File -Force -Filter '*.tmp').Count 'Temporary export files must be removed.'
    } finally { Remove-Item -LiteralPath $parent -Recurse -Force }
}

Invoke-TestCase 'repo export rejects non-Git and unborn roots' {
    $repo = New-TestDirectory
    $archive = "$repo.zip"
    try {
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '-p', $repo, '-o', $archive) } '*committed Git repository*'
        & git -C $repo init --quiet
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $zipProjectScript -Arguments @('--mode', 'repo', '-p', $repo, '-o', $archive) } '*committed Git repository*'
        Assert-PathMissing -Path $archive -Message 'An invalid repo export must not publish a ZIP.'
    } finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

Invoke-TestCase 'review-changes reports clean and rejects non-Git directories' {
    $repo = New-TestGitRepository
    $nonGit = New-TestDirectory
    try {
        Set-Content -LiteralPath (Join-Path $nonGit '.git') -Value 'not-a-gitfile' -Encoding utf8
        $output = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo)
        Assert-Contains $output 'Working tree is clean.' 'Clean repository should be reported.'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $nonGit) } '*not inside a Git working tree*'
    }
    finally {
        Remove-Item -LiteralPath $repo -Recurse -Force
        Remove-Item -LiteralPath $nonGit -Recurse -Force
    }
}

Invoke-TestCase 'review-changes detects staged, unstaged, untracked, deleted, and renamed paths' {
    $repo = New-TestGitRepository -Name 'apollo review space'
    try {
        Copy-Item -LiteralPath (Join-Path $projectRoot '.gitignore') -Destination $repo
        Set-Content -LiteralPath (Join-Path $repo 'staged.txt') -Value 'staged-base' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo 'unstaged.txt') -Value 'unstaged-base' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo 'deleted.txt') -Value 'delete-base' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo 'rename-old.txt') -Value 'rename-base' -Encoding utf8
        & git -C $repo add .
        & git -C $repo commit --quiet -m 'test: add review fixtures'
        Set-Content -LiteralPath (Join-Path $repo 'staged.txt') -Value 'staged-change' -Encoding utf8
        & git -C $repo add staged.txt
        Set-Content -LiteralPath (Join-Path $repo 'unstaged.txt') -Value 'unstaged-change' -Encoding utf8
        Remove-Item -LiteralPath (Join-Path $repo 'deleted.txt')
        & git -C $repo mv rename-old.txt rename-new.txt
        Set-Content -LiteralPath (Join-Path $repo 'untracked.txt') -Value 'untracked' -Encoding utf8
        New-Item -ItemType Directory -Path (Join-Path $repo 'target') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo 'target/ignored.txt') -Value 'ignored' -Encoding utf8
        New-SnapshotFixtureFile -Repository $repo -RelativePath '.plans/private.md'
        New-SnapshotFixtureFile -Repository $repo -RelativePath '.planning/legacy.md'
        New-SnapshotFixtureFile -Repository $repo -RelativePath '.agent-work/private.ps1'
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'artifacts/forced.txt'
        & git -C $repo add -f artifacts/forced.txt

        $summary = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo)
        foreach ($pair in @(@('Staged', 3), @('Unstaged', 2), @('Total', 6))) {
            Assert-True ($summary -match "(?m)^\s*$($pair[0])\s+$($pair[1])\s*$") "Incorrect review count: $($pair[0])."
        }
        Assert-False ($summary.Contains('staged.txt')) 'Default summary must not list individual paths.'
        Assert-Contains $summary 'deleted path(s)' 'Deletion warning missing.'
        $output = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo, '--files')
        foreach ($expected in @('staged.txt', 'unstaged.txt', 'untracked.txt', 'deleted.txt', 'rename-old.txt', 'rename-new.txt', 'Staged', 'Unstaged')) {
            Assert-Contains $output $expected 'Review output is missing expected state.'
        }
        foreach ($line in @('staged.txt [staged]', 'unstaged.txt [unstaged]', 'deleted.txt [unstaged]', 'rename-old.txt -> rename-new.txt [staged]', 'untracked.txt [untracked]')) {
            Assert-Contains $output $line 'Review must report the correct status and index/worktree state.'
        }
        foreach ($path in @('target/ignored.txt', '.plans/private.md', '.planning/legacy.md', '.agent-work/private.ps1')) {
            Assert-False ($output.Contains($path, [System.StringComparison]::OrdinalIgnoreCase)) 'Ignored local files should not surface.'
        }
        Assert-Contains $output 'review before packaging): artifacts/forced.txt' 'Force-tracked generated paths should be flagged.'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo, '--files', '-Detailed') } '*provided more than once*'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo, '--invalid') } 'Unknown option*'
    }
    finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

Invoke-TestCase 'review-changes reports index changes reversed in the worktree' {
    $repo = New-TestGitRepository
    try {
        $path = Join-Path $repo 'seed.txt'
        $original = [System.IO.File]::ReadAllBytes($path)
        Set-Content -LiteralPath $path -Value 'staged change' -Encoding utf8
        & git -C $repo add seed.txt
        [System.IO.File]::WriteAllBytes($path, $original)
        $output = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo, '--files')
        Assert-Contains $output 'seed.txt [staged, unstaged]' 'A reversed staged edit is not a clean worktree.'
        Assert-False ($output.Contains('Working tree is clean.')) 'Index-only changes must not be hidden.'
    }
    finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

Invoke-TestCase 'review flags dependency, CI, large files and both whitespace states' {
    $repo = New-TestGitRepository
    try {
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'src/main.rs'
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'Cargo.toml'
        New-SnapshotFixtureFile -Repository $repo -RelativePath '.github/workflows/ci.yml'
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'other.txt'
        & git -C $repo add .
        & git -C $repo commit --quiet -m 'test: review flags'
        & git -C $repo config core.whitespace blank-at-eol
        Set-Content -LiteralPath (Join-Path $repo 'src/main.rs') -Value 'fn main() {}  '
        & git -C $repo add src/main.rs
        Set-Content -LiteralPath (Join-Path $repo 'other.txt') -Value 'changed  '
        Set-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Value 'changed'
        Set-Content -LiteralPath (Join-Path $repo '.github/workflows/ci.yml') -Value 'changed'
        [System.IO.File]::WriteAllBytes((Join-Path $repo 'large.bin'), [byte[]]::new(1MB))
        $summary = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo)
        foreach ($flag in @('Rust source changed without separate test file changes', 'inline Rust tests', 'Dependency manifests or lock files changed', 'GitHub workflow files changed', 'Large changed files', 'Staged Git whitespace check failed', 'Unstaged Git whitespace check failed')) {
            Assert-Contains $summary $flag 'Expected review flag missing.'
        }
        Assert-False ($summary.Contains('large.bin')) 'Summary must not list paths.'
        $details = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo, '--files')
        Assert-Contains $details 'large.bin (1.0 MiB)' 'Detailed warning must include file size.'
        New-SnapshotFixtureFile -Repository $repo -RelativePath 'tests/behavior.rs'
        $summary = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo)
        Assert-False ($summary.Contains('Rust source changed without separate test file changes')) 'Accompanying test changes should clear advisory.'
    } finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

if (Get-Command just -ErrorAction SilentlyContinue) {
    Invoke-TestCase 'Just forwards zip and review arguments without splitting paths' {
        $parent = New-TestDirectory -Name 'apollo just forwarding'
        $repo = New-TestGitRepository -Name 'apollo just project'
        try {
            Copy-Item -LiteralPath (Join-Path $projectRoot '.zipignore') -Destination $repo
            Push-Location $projectRoot
            try {
                foreach ($mode in @('repo', 'workspace')) {
                    $archive = Join-Path $parent "$mode export.zip"
                    & just zip $mode --project $repo --output $archive
                    if ($LASTEXITCODE -ne 0) { throw "Just zip $mode failed." }
                    Assert-PathExists -Path $archive -Message 'Just did not forward output path.'
                    $manifest = Get-ZipEntryText -Path $archive -EntryName 'EXPORT_MANIFEST.json' | ConvertFrom-Json
                    Assert-Equal $mode $manifest.exportMode 'Just did not forward mode.'
                }
                $review = (& just review --project $repo --files | Out-String)
                Assert-Equal 0 $LASTEXITCODE 'Just review failed.'
                Assert-Contains $review '.zipignore' 'Just did not forward --files.'
                # Native failure should propagate out of the recipe.
                $PSNativeCommandUseErrorActionPreference = $false
                & just zip invalid --project $repo 2>&1 | Out-Null
                Assert-True ($LASTEXITCODE -ne 0) 'Just swallowed a script failure.'
            } finally { Pop-Location }
        } finally { Remove-Item -LiteralPath $parent, $repo -Recurse -Force }
    }

} else { Write-Host 'SKIP Just recipe forwarding (Just not installed; run locally with Just available).' }

Invoke-TestCase 'clean-generated selects Rust and ChatGPT bundle targets with dry run and all' {
    $repo = New-TestGitRepository
    try {
        New-Item -ItemType Directory -Path (Join-Path $repo 'target') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $repo '.dev/chatgpt/bundles') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo 'target/build.bin') -Value 'rust' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo '.dev/chatgpt/bundles/bundle.zip') -Value 'bundle' -Encoding utf8
        $output = Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--rust', '--dry-run')
        Assert-Contains $output 'WOULD REMOVE' 'Dry run should preview Rust cleanup.'
        Assert-PathExists (Join-Path $repo 'target') 'Dry run should preserve target.'
        Assert-PathExists (Join-Path $repo '.dev/chatgpt/bundles') 'Unselected bundles should remain.'
        Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--chatgpt-bundles') | Out-Null
        Assert-PathExists (Join-Path $repo 'target') 'ChatGPT bundle selection should preserve target.'
        Assert-PathMissing (Join-Path $repo '.dev/chatgpt/bundles') 'ChatGPT bundle selection should remove only bundles.'
        New-Item -ItemType Directory -Path (Join-Path $repo '.dev/chatgpt/bundles') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo '.dev/chatgpt/bundles/bundle.zip') -Value 'bundle' -Encoding utf8
        Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--all') | Out-Null
        Assert-PathMissing (Join-Path $repo 'target') 'All should remove Rust target.'
        Assert-PathMissing (Join-Path $repo '.dev/chatgpt/bundles') 'All should remove ChatGPT bundles.'
        $emptyOutput = Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--all')
        Assert-Contains $emptyOutput 'No selected generated output exists.' 'Missing targets should be handled cleanly.'
    }
    finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}


Invoke-TestCase 'clean-generated rejects obsolete npm and Tauri cleanup flags' {
    $repo = New-TestGitRepository
    try {
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--deps') } 'Unknown option*'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--installers') } 'Unknown option*'
    }
    finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

Invoke-TestCase 'clean-generated protects tracked files unless forced' {
    $repo = New-TestGitRepository
    try {
        New-Item -ItemType Directory -Path (Join-Path $repo 'target') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo 'target/tracked.txt') -Value 'tracked' -Encoding utf8
        & git -C $repo add -f target/tracked.txt
        & git -C $repo commit --quiet -m 'test: force-track generated fixture'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--rust') } '*contains tracked files*'
        Invoke-ScriptCapture -ScriptPath $cleanScript -Arguments @('--project', $repo, '--rust', '--force') | Out-Null
        Assert-PathMissing (Join-Path $repo 'target') 'Force should allow selected tracked generated content to be removed.'
    }
    finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

Complete-TestRun
