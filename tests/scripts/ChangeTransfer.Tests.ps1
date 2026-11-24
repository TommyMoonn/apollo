#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
. (Join-Path $PSScriptRoot 'Test.Common.ps1')
. (Join-Path $projectRoot 'scripts/Repository.Common.ps1')
$packageScript = Join-Path $projectRoot 'scripts/package-changes.ps1'
$applyScript = Join-Path $projectRoot 'scripts/apply-changes.ps1'
$restoreScript = Join-Path $projectRoot 'scripts/restore-changes.ps1'
$testAppData = New-TestDirectory -Name 'apollo-transfer-backups'
$originalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = $testAppData
try {
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
            $manifest = Get-ZipEntryText -Path $out -EntryName '.apollo-change-package.json' | ConvertFrom-Json
            Assert-True ($manifest.trackedOnly -is [bool] -and $manifest.trackedOnly) 'Tracked-only mode must be recorded.'
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

    Invoke-TestCase 'package rejects output inside .git and invalid slug' {
        $repo = New-TestGitRepository
        $out = Join-Path (Split-Path $repo -Parent) ((Split-Path $repo -Leaf) + '-wrapper.zip')
        try {
            Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'changed' -Encoding utf8
            Assert-Throws { Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--name', '../unsafe', '--output', $out) } '*Invalid package name*'
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

    Invoke-TestCase 'apply discovery selects newest changed-files ZIP, not snapshots or other ZIPs' {
        $repo = New-TestGitRepository
        $downloads = New-TestDirectory
        try {
            $names = @('apollo-phase-test-changed-files.zip', 'apollo-phase-test-changed-files (1).zip', 'apollo-workspace(2611241935).zip', 'apollo-repo(2611241935).zip', 'apollo-rust-offline-bundle.zip', 'apollo-v0.7.0-windows.zip', 'other-project-changed-files.zip')
            for ($i = 0; $i -lt $names.Count; $i++) {
                $path = Join-Path $downloads $names[$i]
                New-TestZip -Path $path -Entries @{ 'selected.txt' = $names[$i] } | Out-Null
                (Get-Item -LiteralPath $path).LastWriteTime = [DateTime]::new(2025, 1, 1).AddMinutes($i)
            }
            Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--downloads', $downloads) | Out-Null
            Assert-Equal $names[1] (Get-Content -LiteralPath (Join-Path $repo 'selected.txt') -Raw) 'Browser copy should win over older handoff, regardless of newer unrelated ZIPs.'
        } finally {
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
            Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo) | Out-Null
            Assert-PathExists (Join-Path $repo 'remove-dir/file.txt') 'Deleted directory content must be restored.'
        }
        finally {
            Remove-Item -LiteralPath $zipRoot -Recurse -Force
            Remove-Item -LiteralPath $repo -Recurse -Force
        }
    }

    Invoke-TestCase 'apply and restore round trip retains unrelated paths and refuses later edits' {
        $source = New-TestGitRepository
        $targetParent = New-TestDirectory
        $target = Join-Path $targetParent 'target repo'
        $zip = Join-Path $targetParent 'apollo-round-trip-changed-files.zip'
        try {
            Set-Content -LiteralPath (Join-Path $source 'delete.txt') -Value 'original deletion' -NoNewline
            Set-Content -LiteralPath (Join-Path $source 'old.txt') -Value 'rename me' -NoNewline
            Set-Content -LiteralPath (Join-Path $source 'staged.txt') -Value 'original staged' -NoNewline
            & git -C $source add .
            & git -C $source commit --quiet -m 'test: transfer baseline'
            & git clone --quiet -- $source $target
            if ($LASTEXITCODE -ne 0) { throw 'Failed to clone transfer baseline.' }
            $head = (& git -C $source rev-parse HEAD).Trim()
            Set-Content -LiteralPath (Join-Path $source 'seed.txt') -Value 'changed' -NoNewline
            Set-Content -LiteralPath (Join-Path $source 'staged.txt') -Value 'staged' -NoNewline
            & git -C $source add staged.txt
            & git -C $source mv old.txt renamed.txt
            Remove-Item -LiteralPath (Join-Path $source 'delete.txt')
            New-Item -ItemType Directory -Path (Join-Path $source 'new/deep') -Force | Out-Null
            Set-Content -LiteralPath (Join-Path $source 'new/deep/file.txt') -Value 'new' -NoNewline
            Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $source, '--output', $zip) | Out-Null
            $manifest = Get-ZipEntryText -Path $zip -EntryName '.apollo-change-package.json' | ConvertFrom-Json
            Assert-Equal 1 $manifest.schemaVersion 'Schema version.'
            Assert-Equal 'apollo-change-package' $manifest.packageKind 'Package kind.'
            Assert-Equal 'Apollo' $manifest.projectName 'Project identity.'
            Assert-Equal $head $manifest.sourceCommit 'Source HEAD.'
            Assert-True ($manifest.trackedOnly -is [bool] -and -not $manifest.trackedOnly) 'Tracked-only flag must be false.'
            Assert-Equal 4 $manifest.includedFiles 'Payload count.'
            Assert-Equal 2 $manifest.deletedPaths 'Deletion count.'
            Assert-Equal 8 @($manifest.PSObject.Properties).Count 'Exact schema fields.'
            Assert-True ((Get-ZipEntryText -Path $zip -EntryName '.apollo-change-package.json') -match '"createdAt"\s*:\s*"[^"\r\n]+Z"') 'UTC creation time.'
            $preview = Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $target, '--zip', $zip, '--dry-run')
            Assert-Contains $preview 'WOULD DELETE' 'Preview should list deletions.'
            Assert-PathMissing (Join-Path $target 'new') 'Dry run must not create paths.'
            Assert-PathMissing (Join-Path (Get-ApolloImportBase) (Get-RepositorySlug -ProjectRoot $target)) 'Dry run must not create a backup.'
            Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $target, '--zip', $zip) | Out-Null
            Assert-PathMissing (Join-Path $target '.apollo-change-package.json') 'Provenance must not be installed.'
            Assert-PathMissing (Join-Path $target '.chatgpt-delete-manifest.txt') 'Deletion metadata must not be installed.'
            Assert-PathMissing (Join-Path $target 'old.txt') 'Old rename path must be deleted.'
            Assert-PathMissing (Join-Path $target 'delete.txt') 'Deleted path must be absent.'
            Assert-Equal 'rename me' (Get-Content -LiteralPath (Join-Path $target 'renamed.txt') -Raw) 'Rename destination content.'
            Assert-Equal 'staged' (Get-Content -LiteralPath (Join-Path $target 'staged.txt') -Raw) 'Staged changes must transfer.'
            $backup = Get-ChildItem -LiteralPath (Join-Path (Get-ApolloImportBase) (Get-RepositorySlug -ProjectRoot $target)) -Directory | Select-Object -First 1
            Assert-PathExists (Join-Path $backup.FullName 'originals/seed.txt') 'Original content must be backed up.'
            Set-Content -LiteralPath (Join-Path $target 'new/unrelated.txt') -Value 'unrelated' -NoNewline
            Set-Content -LiteralPath (Join-Path $target 'seed.txt') -Value 'later edit' -NoNewline
            Assert-Throws { Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $target, '--backup', $backup.FullName) } '*Restore conflict*'
            Assert-PathMissing (Join-Path $target 'delete.txt') 'Conflict must be detected before restoring any file.'
            Assert-PathExists (Join-Path $target 'new/deep/file.txt') 'Conflict must be detected before removing new files.'
            Set-Content -LiteralPath (Join-Path $target 'seed.txt') -Value 'changed' -NoNewline
            Set-Content -LiteralPath (Join-Path $target 'new/deep/file.txt') -Value 'later new-file edit' -NoNewline
            Assert-Throws { Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $target, '--backup', $backup.FullName) } '*Restore conflict*'
            Assert-Equal 'later new-file edit' (Get-Content -LiteralPath (Join-Path $target 'new/deep/file.txt') -Raw) 'Edited import-created file must be preserved.'
            Set-Content -LiteralPath (Join-Path $target 'new/deep/file.txt') -Value 'new' -NoNewline
            Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $target, '--dry-run') | Out-Null
            Assert-PathMissing (Join-Path $target 'delete.txt') 'Restore dry run must not modify targets.'
            Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $target) | Out-Null
            Assert-PathExists (Join-Path $target 'old.txt') 'Rename source restored.'
            Assert-PathExists (Join-Path $target 'delete.txt') 'Deleted file restored.'
            Assert-PathMissing (Join-Path $target 'renamed.txt') 'Rename destination removed.'
            Assert-PathMissing (Join-Path $target 'new/deep') 'Empty import-created directory removed.'
            Assert-Equal 'unrelated' (Get-Content -LiteralPath (Join-Path $target 'new/unrelated.txt') -Raw) 'Unrelated file preserved.'
            Assert-Equal 'seed' (Get-Content -LiteralPath (Join-Path $target 'seed.txt') -Raw).Trim() 'Original seed restored.'
            Assert-Equal 'original staged' (Get-Content -LiteralPath (Join-Path $target 'staged.txt') -Raw) 'Original staged content restored.'
            Set-Content -LiteralPath (Join-Path $target 'seed.txt') -Value 'after restore' -NoNewline
            Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $target, '--backup', $backup.FullName) | Out-Null
            Assert-Equal 'after restore' (Get-Content -LiteralPath (Join-Path $target 'seed.txt') -Raw) 'Completed backup must not replay over later edits.'
        } finally {
            Remove-Item -LiteralPath $source -Recurse -Force
            Remove-Item -LiteralPath $targetParent -Recurse -Force
        }
    }

    Invoke-TestCase 'apply no-backup suppresses backup creation' {
        $repo = New-TestGitRepository
        $zipRoot = New-TestDirectory
        $localAppData = New-TestDirectory -Name 'apollo-localappdata'
        $zip = Join-Path $zipRoot 'no-backup.zip'
        $oldLocalAppData = $env:LOCALAPPDATA
        try {
            $env:LOCALAPPDATA = $localAppData
            New-TestZip -Path $zip -Entries @{ 'seed.txt' = 'after' } | Out-Null
            Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip, '--no-backup') | Out-Null
            Assert-Equal 'after' (Get-Content -LiteralPath (Join-Path $repo 'seed.txt') -Raw) 'Apply should update the target.'
            Assert-PathMissing (Join-Path $localAppData 'Apollo/ChatGPTImports') 'No-backup should not create backup roots.'
        }
        finally {
            $env:LOCALAPPDATA = $oldLocalAppData
            Remove-Item -LiteralPath $zipRoot -Recurse -Force
            Remove-Item -LiteralPath $localAppData -Recurse -Force
            Remove-Item -LiteralPath $repo -Recurse -Force
        }
    }

    Invoke-TestCase 'restore-changes supports legacy backups, dry run and missing backup failure' {
        $repo = New-TestGitRepository
        $backup = New-TestDirectory -Name 'apollo-backup'
        try {
            New-Item -ItemType Directory -Path (Join-Path $backup 'nested') -Force | Out-Null
            Set-Content -LiteralPath (Join-Path $backup 'nested/file.txt') -Value 'restored' -NoNewline -Encoding utf8
            $output = Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo, '--backup', $backup, '--dry-run')
            Assert-Contains $output 'WOULD' 'Restore dry run should preview work.'
            Assert-PathMissing (Join-Path $repo 'nested/file.txt') 'Restore dry run should not write.'
            Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo, '--backup', $backup) | Out-Null
            Assert-Equal 'restored' (Get-Content -LiteralPath (Join-Path $repo 'nested/file.txt') -Raw) 'Legacy restore should restore files and directories.'
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

    Invoke-TestCase 'clean and deletion-only package behavior; force failure preserves existing bytes' {
        $repo = New-TestGitRepository
        $folder = New-TestDirectory
        $out = Join-Path $folder 'package.zip'
        try {
            Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--output', $out) | Out-Null
            Assert-PathMissing $out 'Clean tree should not produce a package.'
            Remove-Item -LiteralPath (Join-Path $repo 'seed.txt')
            Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--output', $out) | Out-Null
            $manifest = Get-ZipEntryText -Path $out -EntryName '.apollo-change-package.json' | ConvertFrom-Json
            Assert-Equal 0 $manifest.includedFiles 'Deletion-only package has no payload.'
            Assert-Equal 1 $manifest.deletedPaths 'Deletion-only package count.'
            $hash = (Get-FileHash -LiteralPath $out).Hash
            Set-Content -LiteralPath (Join-Path $repo '.apollo-change-package.json') -Value '{}'
            Assert-Throws { Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--output', $out, '--force') } '*Reserved package metadata*'
            Assert-Equal $hash (Get-FileHash -LiteralPath $out).Hash 'Force must preserve output on failure.'
            Remove-Item -LiteralPath (Join-Path $repo '.apollo-change-package.json')
            if ($IsWindows) {
                $locked = [IO.File]::Open($out, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
                try { Assert-Throws { Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--output', $out, '--force') } '*' }
                finally { $locked.Dispose() }
                Assert-Equal $hash (Get-FileHash -LiteralPath $out).Hash 'A failed publication must preserve the old ZIP.'
                Assert-Equal 1 @(Get-ChildItem -LiteralPath $folder -Force).Count 'Temporary archive must be cleaned up.'
            }
            & git -C $repo restore -- seed.txt
            Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $out) | Out-Null
            Assert-PathMissing (Join-Path $repo 'seed.txt') 'New-format deletion-only package must apply.'
        } finally {
            Remove-Item -LiteralPath $folder -Recurse -Force
            Remove-Item -LiteralPath $repo -Recurse -Force
        }
    }

    Invoke-TestCase 'package omits local-only paths even when the source ignore rules are incomplete' {
        $repo = New-TestGitRepository
        $folder = New-TestDirectory
        $out = Join-Path $folder 'package.zip'
        try {
            foreach ($path in @('.plans/work.md', '.project/local.md', '.agent-work/offline.ps1', '.agents/skills/custom/SKILL.md', 'target/build.txt', '.env')) {
                $file = Join-Path $repo $path
                New-Item -ItemType Directory -Path (Split-Path $file -Parent) -Force | Out-Null
                Set-Content -LiteralPath $file -Value 'local only'
            }
            Set-Content -LiteralPath (Join-Path $repo 'seed.txt') -Value 'changed'
            Invoke-ScriptCapture -ScriptPath $packageScript -Arguments @('--project', $repo, '--output', $out) | Out-Null
            Assert-Equal @('.apollo-change-package.json','seed.txt') @((Get-ZipEntryNames -Path $out) | Sort-Object) 'Only eligible payload and metadata should be present.'
        } finally {
            Remove-Item -LiteralPath $folder -Recurse -Force
            Remove-Item -LiteralPath $repo -Recurse -Force
        }
    }

    Invoke-TestCase 'invalid provenance, snapshot and path conflicts fail before target mutation' {
        $repo = New-TestGitRepository
        $folder = New-TestDirectory
        $zip = Join-Path $folder 'package.zip'
        try {
            $valid = [ordered]@{schemaVersion=1; packageKind='apollo-change-package'; projectName='Apollo'; sourceCommit=('0'*40); trackedOnly=$false; includedFiles=1; deletedPaths=0; createdAt='2026-11-24T00:00:00Z'}
            foreach ($case in @(
                @{ Key='schemaVersion'; Value=2 }, @{ Key='packageKind'; Value='other-change-package' },
                @{ Key='projectName'; Value='Archeion' }, @{ Key='sourceCommit'; Value='bad' },
                @{ Key='trackedOnly'; Value='false' }, @{ Key='includedFiles'; Value=2 },
                @{ Key='deletedPaths'; Value=-1 }, @{ Key='createdAt'; Value='2026-11-24T00:00:00+07:00' }
            )) {
                $manifest = [ordered]@{}; foreach ($key in $valid.Keys) { $manifest[$key] = $valid[$key] }
                $manifest[$case.Key] = $case.Value
                New-TestZip -Path $zip -Entries @{'.apollo-change-package.json'=($manifest | ConvertTo-Json); 'new.txt'='value'} | Out-Null
                Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*provenance*'
                Assert-PathMissing (Join-Path $repo 'new.txt') 'Invalid provenance must not write targets.'
            }
            foreach ($entries in @(
                @{'.apollo-change-package.json'='{'; 'new.txt'='value'},
                @{'.apollo-change-package.json'=('{"schemaVersion":1,' + (($valid | ConvertTo-Json -Compress).Substring(1))); 'new.txt'='value'},
                @{'.apollo-change-package.json'=($valid | ConvertTo-Json); 'nested/.apollo-change-package.json'=($valid | ConvertTo-Json); 'new.txt'='value'},
                @{'.archeion-change-package.json'='{}'; 'new.txt'='value'},
                @{'EXPORT_MANIFEST.json'='{}'; 'new.txt'='value'},
                @{'new.txt'='value'; '.plans/work.md'='local'},
                @{'new.txt'='value'; 'new.txt/child'='conflict'},
                @{'new.txt'='value'; 'nested/file'='a'; 'NESTED/other'='b'},
                @{'new.txt'='value'; '.chatgpt-delete-manifest.txt'="new.txt`n"},
                @{'new.txt'='value'; 'seed.txt/child'='parent is file'},
                @{'new.txt'='value'; '.chatgpt-delete-manifest.txt'="nested/file`nnested\\file"}
            )) {
                New-TestZip -Path $zip -Entries $entries | Out-Null
                Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*'
                Assert-PathMissing (Join-Path $repo 'new.txt') 'Unsafe input must not apply any payload.'
            }
            New-TestZip -Path $zip -Entries @{ 'wrapper/.apollo-change-package.json'=($valid | ConvertTo-Json); 'wrapper/new.txt'='value' } | Out-Null
            $output = Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip)
            Assert-Contains $output 'Source commit is not present' 'Missing source commit should warn, not refuse.'
            Assert-PathExists (Join-Path $repo 'new.txt') 'Wrapped new-format package should import.'
            Assert-PathMissing (Join-Path $repo 'wrapper') 'Wrapper must not be installed.'
        } finally {
            Remove-Item -LiteralPath $folder -Recurse -Force
            Remove-Item -LiteralPath $repo -Recurse -Force
        }
    }

    Invoke-TestCase 'linked targets and linked ZIP entries fail without touching unrelated content' {
        $repo = New-TestGitRepository
        $outside = New-TestDirectory
        $folder = New-TestDirectory
        $zip = Join-Path $folder 'linked.zip'
        try {
            $link = Join-Path $repo 'linked'
            if ($IsWindows) { New-Item -ItemType Junction -Path $link -Target $outside | Out-Null }
            else { New-Item -ItemType SymbolicLink -Path $link -Target $outside | Out-Null }
            New-TestZip -Path $zip -Entries @{'new.txt'='value'; 'linked/unsafe.txt'='outside'} | Out-Null
            Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*reparse points*'
            Assert-PathMissing (Join-Path $repo 'new.txt') 'Linked target must fail preflight.'
            Assert-PathMissing (Join-Path $outside 'unsafe.txt') 'Outside target must not be changed.'
            New-TestZip -Path $zip -Entries @{'new.txt'='value'; 'link.txt'='target'} | Out-Null
            $archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Update)
            try { $archive.GetEntry('link.txt').ExternalAttributes = -1610612736 } finally { $archive.Dispose() }
            Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*linked or unsupported*'
            Assert-PathMissing (Join-Path $repo 'new.txt') 'Linked archive entry must fail before writing.'
        } finally {
            if (Test-Path -LiteralPath (Join-Path $repo 'linked')) { Remove-Item -LiteralPath (Join-Path $repo 'linked') -Force }
            Remove-Item -LiteralPath $repo -Recurse -Force
            Remove-Item -LiteralPath $outside -Recurse -Force
            Remove-Item -LiteralPath $folder -Recurse -Force
        }
    }

    Invoke-TestCase 'wrapped legacy deletion-only handoff is supported' {
        $repo = New-TestGitRepository
        $folder = New-TestDirectory
        $zip = Join-Path $folder 'wrapped.zip'
        try {
            New-TestZip -Path $zip -Entries @{'wrapper/.chatgpt-delete-manifest.txt'='seed.txt'} | Out-Null
            Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) | Out-Null
            Assert-PathMissing (Join-Path $repo 'seed.txt') 'Wrapped deletion-only archive must delete the intended path.'
            Assert-PathMissing (Join-Path $repo 'wrapper') 'Wrapper must not be installed.'
            Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo) | Out-Null
            Assert-PathExists (Join-Path $repo 'seed.txt') 'Deletion-only import must be reversible.'
        } finally {
            Remove-Item -LiteralPath $repo -Recurse -Force
            Remove-Item -LiteralPath $folder -Recurse -Force
        }
    }

    if ($IsWindows) {
        Invoke-TestCase 'interrupted apply and restore retain usable journal and allow retry' {
            $repo = New-TestGitRepository
            $folder = New-TestDirectory
            $zip = Join-Path $folder 'package.zip'
            try {
                New-TestZip -Path $zip -Entries @{'a.txt'='new'; 'seed.txt'='changed'} | Out-Null
                $seed = Join-Path $repo 'seed.txt'
                $locked = [IO.File]::Open($seed, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
                try { Assert-Throws { Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) } '*' }
                finally { $locked.Dispose() }
                Assert-PathExists (Join-Path $repo 'a.txt') 'Earlier operation should have completed before locked-file failure.'
                Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo) | Out-Null
                Assert-PathMissing (Join-Path $repo 'a.txt') 'Restore should undo completed operations after partial apply.'
                Assert-Equal 'seed' (Get-Content -LiteralPath $seed -Raw).Trim() 'Failed copy must leave original intact.'
                Invoke-ScriptCapture -ScriptPath $applyScript -Arguments @('--project', $repo, '--zip', $zip) | Out-Null
                $locked = [IO.File]::Open($seed, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
                try { Assert-Throws { Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo) } '*' }
                finally { $locked.Dispose() }
                Assert-PathMissing (Join-Path $repo 'a.txt') 'Earlier restore operation should have completed.'
                Invoke-ScriptCapture -ScriptPath $restoreScript -Arguments @('--project', $repo) | Out-Null
                Assert-Equal 'seed' (Get-Content -LiteralPath $seed -Raw).Trim() 'Retry should restore the remaining original.'
            } finally {
                Remove-Item -LiteralPath $folder -Recurse -Force
                Remove-Item -LiteralPath $repo -Recurse -Force
            }
        }
    }
    if (Get-Command just -ErrorAction SilentlyContinue) {
        Invoke-TestCase 'Just forwards package, apply and restore flags and paths containing spaces' {
            $repo = New-TestGitRepository -Name 'apollo transfer project'
            $folder = New-TestDirectory -Name 'apollo transfer exports'
            $zip = Join-Path $folder 'changed files.zip'
            try {
                Set-Content -LiteralPath (Join-Path $repo 'new.txt') -Value 'value' -NoNewline
                Push-Location $projectRoot
                try {
                    & just package transfer --project $repo --output $zip
                    Assert-Equal 0 $LASTEXITCODE 'Just package failed.'
                    Assert-PathExists $zip 'Just must forward output path.'
                    Remove-Item -LiteralPath (Join-Path $repo 'new.txt')
                    & just apply --project $repo --zip $zip --dry-run
                    Assert-Equal 0 $LASTEXITCODE 'Just apply dry run failed.'
                    Assert-PathMissing (Join-Path $repo 'new.txt') 'Just must forward dry-run.'
                    & just apply-file $zip --project $repo
                    Assert-Equal 0 $LASTEXITCODE 'Just apply-file failed.'
                    Assert-PathExists (Join-Path $repo 'new.txt') 'Just must forward archive path.'
                    & just restore --project $repo --dry-run
                    Assert-Equal 0 $LASTEXITCODE 'Just restore dry run failed.'
                    Assert-PathExists (Join-Path $repo 'new.txt') 'Restore preview must not remove the file.'
                    & just restore --project $repo
                    Assert-Equal 0 $LASTEXITCODE 'Just restore failed.'
                    Assert-PathMissing (Join-Path $repo 'new.txt') 'Just must forward restore project path.'
                } finally { Pop-Location }
            } finally {
                Remove-Item -LiteralPath $repo, $folder -Recurse -Force
            }
        }
    } else { Write-Host 'SKIP Just transfer forwarding (Just not installed).' }
} finally {
    $env:LOCALAPPDATA = $originalAppData
    Remove-Item -LiteralPath $testAppData -Recurse -Force
}
Complete-TestRun
