#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
. (Join-Path $PSScriptRoot 'Test.Common.ps1')
. (Join-Path $projectRoot 'scripts/Repository.Common.ps1')

$zipProjectScript = Join-Path $projectRoot 'scripts/zip-project.ps1'
$reviewScript = Join-Path $projectRoot 'scripts/review-changes.ps1'
$packageScript = Join-Path $projectRoot 'scripts/package-changes.ps1'
$packageWrapper = Join-Path $projectRoot 'scripts/package-changed-files.ps1'
$applyScript = Join-Path $projectRoot 'scripts/apply-changes.ps1'
$applyWrapper = Join-Path $projectRoot 'scripts/apply-chatgpt-zip.ps1'
$restoreScript = Join-Path $projectRoot 'scripts/restore-changes.ps1'
$restoreWrapper = Join-Path $projectRoot 'scripts/restore-chatgpt-import.ps1'
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

Invoke-TestCase 'zip-project uses real exclusions and preserves required source and local context' {
    $parent = New-TestDirectory -Name 'apollo-zip-parent'
    $repo = Join-Path $parent 'Apollo Sample'
    try {
        Copy-SnapshotFixtureScripts -Repository $repo
        Copy-Item -LiteralPath (Join-Path $projectRoot '.zipignore') -Destination $repo
        $included = @('README.md', 'Cargo.toml', 'Cargo.lock', 'src/main.rs', 'assets/icon.png', 'tests/sample.rs',
            '.plans/plan.md', '.planning/legacy.md', '.project/review.md', '.agents/skills/example/SKILL.md', 'docs/space name.md')
        $excluded = @('.git/config', 'target/build.txt', 'artifacts/result.txt', '.dev/chatgpt/bundles/bundle.txt',
            '.scratch/cache.txt', '.agent-work/prepare.ps1', 'nested/.env.local', '.env', 'private.pem',
            'nested/run.log', 'apollo-cargo-vendor-offline-bundle.part-000.zip', 'temporary.tmp', 'Desktop.ini')
        foreach ($path in ($included + $excluded)) { New-SnapshotFixtureFile -Repository $repo -RelativePath $path }

        Invoke-ScriptCapture -ScriptPath (Join-Path $repo 'scripts/zip-project.ps1') | Out-Null
        $archives = @(Get-ChildItem -LiteralPath $parent -File -Filter 'apollo-sample(*).zip')
        Assert-Equal 1 $archives.Count 'Expected one project snapshot.'
        Assert-True ($archives[0].Name -match '^apollo-sample\(\d{10}\)\.zip$') 'Snapshot filename should use slug and yyMMddHHmm timestamp.'
        $entries = @(Get-ZipEntryNames -Path $archives[0].FullName)
        foreach ($path in ($included + @('scripts/zip-project.ps1', '.zipignore'))) {
            Assert-True ($entries -ccontains $path) "Required project-relative entry missing: $path"
        }
        foreach ($path in $excluded) { Assert-False ($entries -ccontains $path) "Excluded entry leaked: $path" }
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
        Invoke-ScriptCapture -ScriptPath (Join-Path $repo 'scripts/zip-project.ps1') | Out-Null
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
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy } '*.zipignore*'
        '!secret.txt' | Set-Content -LiteralPath (Join-Path $repo '.zipignore')
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $scriptCopy } '*negation is not supported*'
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
            Assert-Throws { Invoke-ScriptCapture -ScriptPath (Join-Path $repo 'scripts/zip-project.ps1') } '*locked.txt*'
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

        $output = Invoke-ScriptCapture -ScriptPath $reviewScript -Arguments @('--project', $repo, '--files')
        foreach ($expected in @('staged.txt', 'unstaged.txt', 'untracked.txt', 'deleted.txt', 'rename-old.txt', 'rename-new.txt', 'Staged', 'Unstaged')) {
            Assert-Contains $output $expected 'Review output is missing expected state.'
        }
        foreach ($line in @('M staged.txt [staged]', 'M unstaged.txt [unstaged]', 'D deleted.txt [unstaged]', 'R rename-old.txt -> rename-new.txt [staged]', '? untracked.txt [untracked]')) {
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
        Assert-Contains $output '~ seed.txt [staged, unstaged]' 'A reversed staged edit is not a clean worktree.'
        Assert-False ($output.Contains('Working tree is clean.')) 'Index-only changes must not be hidden.'
    }
    finally { Remove-Item -LiteralPath $repo -Recurse -Force }
}

Invoke-TestCase 'package-changes includes tracked and untracked files while excluding ignored output' {
    $repo = New-TestGitRepository
    $out = Join-Path (Split-Path $repo -Parent) ((Split-Path $repo -Leaf) + '-package.zip')
    try {
        New-Item -ItemType Directory -Path (Join-Path $repo 'src') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo 'src/value.txt') -Value 'base' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo '.gitignore') -Value '/target/' -Encoding utf8
        & git -C $repo add .
        & git -C $repo commit --quiet -m 'test: add package fixtures'
        Set-Content -LiteralPath (Join-Path $repo 'src/value.txt') -Value 'changed' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo 'new.txt') -Value 'new' -Encoding utf8
        New-Item -ItemType Directory -Path (Join-Path $repo 'target') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo 'target/ignored.txt') -Value 'ignored' -Encoding utf8

        Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--name', 'phase-test', '--output', $out) | Out-Null
        $entries = @(Get-ZipEntryNames -Path $out)
        Assert-True ($entries -contains 'src/value.txt') 'Modified tracked file should be packaged with project-relative path.'
        Assert-True ($entries -contains 'new.txt') 'Untracked file should be packaged by default.'
        Assert-False ($entries -contains 'target/ignored.txt') 'Ignored target output should not be packaged.'
        Assert-False ($entries -contains '.chatgpt-delete-manifest.txt') 'Deletion manifest should not exist without deletions.'
    }
    finally {
        if (Test-Path $out) { Remove-Item $out -Force }
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'package-changes tracked-only omits untracked files and protects existing output' {
    $repo = New-TestGitRepository
    $out = Join-Path (Split-Path $repo -Parent) ((Split-Path $repo -Leaf) + '-tracked.zip')
    try {
        Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'changed' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo 'untracked.txt') -Value 'untracked' -Encoding utf8
        Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--tracked-only', '--output', $out) | Out-Null
        Assert-False ((Get-ZipEntryNames -Path $out) -contains 'untracked.txt') 'Tracked-only package should omit untracked files.'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--tracked-only', '--output', $out) } '*Output already exists*'
        Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--tracked-only', '--output', $out, '--force') | Out-Null
        Assert-PathExists $out 'Force should replace existing package.'
    }
    finally {
        if (Test-Path $out) { Remove-Item $out -Force }
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'package-changes records deletions and renames in the manifest' {
    $repo = New-TestGitRepository
    $out = Join-Path (Split-Path $repo -Parent) ((Split-Path $repo -Leaf) + '-delete.zip')
    try {
        Set-Content -LiteralPath (Join-Path $repo 'delete-me.txt') -Value 'delete' -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo 'rename-old.txt') -Value 'rename' -Encoding utf8
        & git -C $repo add .
        & git -C $repo commit --quiet -m 'test: add delete fixtures'
        Remove-Item -LiteralPath (Join-Path $repo 'delete-me.txt')
        & git -C $repo mv rename-old.txt rename-new.txt

        Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--output', $out) | Out-Null
        $entries = @(Get-ZipEntryNames -Path $out)
        Assert-True ($entries -contains 'rename-new.txt') 'Renamed destination should be included.'
        $manifest = Get-ZipEntryText -Path $out -EntryName '.chatgpt-delete-manifest.txt'
        Assert-Contains $manifest 'delete-me.txt' 'Deleted file should be recorded.'
        Assert-Contains $manifest 'rename-old.txt' 'Renamed source should be recorded as a deletion.'
    }
    finally {
        if (Test-Path $out) { Remove-Item $out -Force }
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'package wrapper forwards arguments and package refuses .git output' {
    $repo = New-TestGitRepository
    $out = Join-Path (Split-Path $repo -Parent) ((Split-Path $repo -Leaf) + '-wrapper.zip')
    try {
        Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'changed' -Encoding utf8
        Invoke-ScriptCapture -ScriptPath $packageWrapper -Arguments @('--project', $repo, '--output', $out) | Out-Null
        Assert-PathExists $out 'Compatibility wrapper should create the package.'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--output', (Join-Path $repo '.git/unsafe.zip')) } '*.git*'
    }
    finally {
        if (Test-Path $out) { Remove-Item $out -Force }
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes handles explicit ZIP, unchanged files, changes, and new files' {
    $repo = New-TestGitRepository
    $zip = Join-Path (New-TestDirectory) 'explicit.zip'
    try {
        Set-Content -LiteralPath (Join-Path $repo 'same.txt') -Value 'same' -NoNewline -Encoding utf8
        Set-Content -LiteralPath (Join-Path $repo 'changed.txt') -Value 'old' -NoNewline -Encoding utf8
        & git -C $repo add .
        & git -C $repo commit --quiet -m 'test: add apply fixtures'
        New-TestZip -Path $zip -Entries @{ 'same.txt' = 'same'; 'changed.txt' = 'new'; 'new.txt' = 'created' } | Out-Null
        $output = Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip)
        Assert-Contains $output 'UNCHANGED' 'Identical file should be skipped.'
        Assert-Equal 'new' (Get-Content -LiteralPath (Join-Path $repo 'changed.txt') -Raw) 'Changed file should be overwritten.'
        Assert-Equal 'created' (Get-Content -LiteralPath (Join-Path $repo 'new.txt') -Raw) 'New file should be created.'
    }
    finally {
        Remove-Item -LiteralPath (Split-Path $zip -Parent) -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes automatic discovery chooses newest project ZIP' {
    $repo = New-TestGitRepository
    $downloads = New-TestDirectory
    try {
        $slug = Get-RepositorySlug -ProjectRoot $repo
        $changedFilesZip = Join-Path $downloads "$slug-phase-test-changed-files.zip"
        $snapshotZip = Join-Path $downloads "$slug(2608281142).zip"
        $snapshotDuplicateZip = Join-Path $downloads "$slug(2608281142) (1).zip"
        $unrelatedZip = Join-Path $downloads 'other-project(2608281200).zip'
        New-TestZip -Path $changedFilesZip -Entries @{ 'selected.txt' = 'changed-files' } | Out-Null
        New-TestZip -Path $snapshotZip -Entries @{ 'selected.txt' = 'snapshot' } | Out-Null
        New-TestZip -Path $snapshotDuplicateZip -Entries @{ 'selected.txt' = 'snapshot-copy' } | Out-Null
        New-TestZip -Path $unrelatedZip -Entries @{ 'selected.txt' = 'unrelated' } | Out-Null
        (Get-Item $changedFilesZip).LastWriteTime = (Get-Date).AddMinutes(-3)
        (Get-Item $snapshotZip).LastWriteTime = (Get-Date).AddMinutes(-2)
        (Get-Item $snapshotDuplicateZip).LastWriteTime = (Get-Date).AddMinutes(-1)
        (Get-Item $unrelatedZip).LastWriteTime = Get-Date
        Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--downloads', $downloads) | Out-Null
        Assert-Equal 'snapshot-copy' (Get-Content -LiteralPath (Join-Path $repo 'selected.txt') -Raw) 'Newest project-prefixed ZIP should be selected regardless of package shape or browser duplicate suffix.'
    }
    finally {
        Remove-Item -LiteralPath $downloads -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}


Invoke-TestCase 'apply-changes strips a redundant single archive root automatically' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $zip = Join-Path $zipRoot 'wrapped.zip'
    try {
        New-TestZip -Path $zip -Entries @{ 'wrapper/seed.txt' = 'wrapped-change' } | Out-Null
        Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) | Out-Null
        Assert-Equal 'wrapped-change' (Get-Content -LiteralPath (Join-Path $repo 'seed.txt') -Raw) 'Single redundant archive root should be stripped.'
        Assert-PathMissing (Join-Path $repo 'wrapper/seed.txt') 'Redundant root should not be copied into the project.'
    }
    finally {
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes dry run makes no changes' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $zip = Join-Path $zipRoot 'dry.zip'
    try {
        New-TestZip -Path $zip -Entries @{ 'dry.txt' = 'value' } | Out-Null
        $output = Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip, '--dry-run')
        Assert-Contains $output 'WOULD COPY' 'Dry run should preview copy.'
        Assert-PathMissing (Join-Path $repo 'dry.txt') 'Dry run should not write files.'
    }
    finally {
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes rejects traversal, absolute paths, and .git entries' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    try {
        foreach ($case in @(
            @{ Name = 'traversal.zip'; Entry = '../outside.txt'; Pattern = '*traversal*' },
            @{ Name = 'absolute.zip'; Entry = '/absolute.txt'; Pattern = '*absolute path*' },
            @{ Name = 'drive-relative.zip'; Entry = 'C:outside.txt'; Pattern = '*absolute path*' },
            @{ Name = 'git.zip'; Entry = '.git/config'; Pattern = '*.git*' }
        )) {
            $zip = Join-Path $zipRoot $case.Name
            New-TestZip -Path $zip -Entries @{ $case.Entry = 'unsafe' } | Out-Null
            Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } $case.Pattern
        }
    }
    finally {
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes rejects tracked dirty state unless explicitly allowed' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $zip = Join-Path $zipRoot 'dirty.zip'
    try {
        Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'dirty' -Encoding utf8
        New-TestZip -Path $zip -Entries @{ 'new.txt' = 'value' } | Out-Null
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*tracked changes*'
        Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip, '--allow-dirty') | Out-Null
        Assert-PathExists (Join-Path $repo 'new.txt') 'Allow-dirty should permit the import.'
    }
    finally {
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes applies file deletion manifest' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $zip = Join-Path $zipRoot 'delete.zip'
    try {
        Set-Content -LiteralPath (Join-Path $repo 'remove.txt') -Value 'remove' -Encoding utf8
        & git -C $repo add remove.txt
        & git -C $repo commit --quiet -m 'test: add removal fixture'
        New-TestZip -Path $zip -Entries @{ '.chatgpt-delete-manifest.txt' = 'remove.txt' } | Out-Null
        Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) | Out-Null
        Assert-PathMissing (Join-Path $repo 'remove.txt') 'Manifest should delete the file.'
    }
    finally {
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes preflights unsafe deletions before copying files' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $zip = Join-Path $zipRoot 'preflight.zip'
    try {
        New-Item -ItemType Directory -Path (Join-Path $repo 'remove-dir') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo 'remove-dir/file.txt') -Value 'remove' -Encoding utf8
        & git -C $repo add remove-dir/file.txt
        & git -C $repo commit --quiet -m 'test: add preflight fixture'
        New-TestZip -Path $zip -Entries @{
            'copied-before-error.txt' = 'must-not-appear'
            '.chatgpt-delete-manifest.txt' = 'dir:remove-dir'
        } | Out-Null

        Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*--allow-directory-deletion*'
        Assert-PathMissing (Join-Path $repo 'copied-before-error.txt') 'Preflight failure must happen before any copy is applied.'
        Assert-PathExists (Join-Path $repo 'remove-dir/file.txt') 'Preflight failure must preserve the requested deletion target.'
    }
    finally {
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply-changes requires opt-in for directory deletion' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $zip = Join-Path $zipRoot 'dir-delete.zip'
    try {
        New-Item -ItemType Directory -Path (Join-Path $repo 'remove-dir') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $repo 'remove-dir/file.txt') -Value 'remove' -Encoding utf8
        & git -C $repo add remove-dir/file.txt
        & git -C $repo commit --quiet -m 'test: add directory fixture'
        New-TestZip -Path $zip -Entries @{ '.chatgpt-delete-manifest.txt' = 'dir:remove-dir' } | Out-Null
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*--allow-directory-deletion*'
        Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip, '--allow-directory-deletion') | Out-Null
        Assert-PathMissing (Join-Path $repo 'remove-dir') 'Explicit opt-in should allow directory deletion.'
    }
    finally {
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply backup can be restored by explicit and newest backup selection' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $localAppData = New-TestDirectory -Name 'apollo-localappdata'
    $zip = Join-Path $zipRoot 'backup.zip'
    $oldLocalAppData = $env:LOCALAPPDATA
    try {
        $env:LOCALAPPDATA = $localAppData
        Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'before' -NoNewline -Encoding utf8
        & git -C $repo add seed.txt
        & git -C $repo commit --quiet -m 'test: update seed fixture'
        New-TestZip -Path $zip -Entries @{ 'seed.txt' = 'after' } | Out-Null
        Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) | Out-Null
        $slug = Get-RepositorySlug -ProjectRoot $repo
        $backupBase = Join-Path (Join-Path $localAppData 'ChatGPTImports') $slug
        $backup = Get-ChildItem -LiteralPath $backupBase -Directory | Sort-Object Name -Descending | Select-Object -First 1
        Assert-True ($null -ne $backup) 'Apply should create a backup for overwritten files.'
        Assert-Equal 'before' (Get-Content -LiteralPath (Join-Path $backup.FullName 'seed.txt') -Raw) 'Backup should contain original content.'

        Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'modified-again' -NoNewline -Encoding utf8
        Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo, '--backup', $backup.FullName) | Out-Null
        Assert-Equal 'before' (Get-Content -LiteralPath (Join-Path $repo 'seed.txt') -Raw) 'Explicit restore should reinstate backup content.'

        Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'modified-third' -NoNewline -Encoding utf8
        Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo) | Out-Null
        Assert-Equal 'before' (Get-Content -LiteralPath (Join-Path $repo 'seed.txt') -Raw) 'Newest backup discovery should restore content.'
    }
    finally {
        $env:LOCALAPPDATA = $oldLocalAppData
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $localAppData -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'apply no-backup suppresses backup creation and wrapper forwards' {
    $repo = New-TestGitRepository
    $zipRoot = New-TestDirectory
    $localAppData = New-TestDirectory -Name 'apollo-localappdata'
    $zip = Join-Path $zipRoot 'no-backup.zip'
    $oldLocalAppData = $env:LOCALAPPDATA
    try {
        $env:LOCALAPPDATA = $localAppData
        New-TestZip -Path $zip -Entries @{ 'seed.txt' = 'after' } | Out-Null
        Invoke-ScriptCapture -ScriptPath $applyWrapper -Arguments @('--project', $repo, '--zip', $zip, '--no-backup') | Out-Null
        Assert-Equal 'after' (Get-Content -LiteralPath (Join-Path $repo 'seed.txt') -Raw) 'Apply wrapper should forward arguments.'
        Assert-PathMissing (Join-Path $localAppData 'ChatGPTImports') 'No-backup should not create backup roots.'
    }
    finally {
        $env:LOCALAPPDATA = $oldLocalAppData
        Remove-Item -LiteralPath $zipRoot -Recurse -Force
        Remove-Item -LiteralPath $localAppData -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'restore-changes supports dry run, directories, missing backup failure, and wrapper' {
    $repo = New-TestGitRepository
    $backup = New-TestDirectory -Name 'apollo-backup'
    try {
        New-Item -ItemType Directory -Path (Join-Path $backup 'nested') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $backup 'nested/file.txt') -Value 'restored' -NoNewline -Encoding utf8
        $output = Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo, '--backup', $backup, '--dry-run')
        Assert-Contains $output 'WOULD' 'Restore dry run should preview work.'
        Assert-PathMissing (Join-Path $repo 'nested/file.txt') 'Restore dry run should not write.'
        Invoke-ScriptCapture -ScriptPath $restoreWrapper -Arguments @('--project', $repo, '--backup', $backup) | Out-Null
        Assert-Equal 'restored' (Get-Content -LiteralPath (Join-Path $repo 'nested/file.txt') -Raw) 'Restore wrapper should restore files and directories.'
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo, '--backup', (Join-Path $backup 'missing')) } '*does not exist*'
    }
    finally {
        Remove-Item -LiteralPath $backup -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

Invoke-TestCase 'restore refuses .git backup content' {
    $repo = New-TestGitRepository
    $backup = New-TestDirectory -Name 'apollo-backup'
    try {
        New-Item -ItemType Directory -Path (Join-Path $backup '.git') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $backup '.git/config') -Value 'unsafe' -Encoding utf8
        Assert-Throws { Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo, '--backup', $backup) } '*.git*'
    }
    finally {
        Remove-Item -LiteralPath $backup -Recurse -Force
        Remove-Item -LiteralPath $repo -Recurse -Force
    }
}

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
