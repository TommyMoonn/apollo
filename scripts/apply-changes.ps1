#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$defaultProject = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$defaultDownloads = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)) 'Downloads'
$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    zip = @{ Aliases = @('-ZipPath') }
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = $defaultProject }
    downloads = @{ Aliases = @('-DownloadsPath'); Default = $defaultDownloads }
    pattern = @{ Aliases = @('-ZipPattern') }
    'strip-root' = @{ Kind = 'Switch'; Aliases = @('-StripSingleRoot') }
    'dry-run' = @{ Kind = 'Switch'; Aliases = @('-n', '-DryRun') }
    'allow-dirty' = @{ Kind = 'Switch'; Aliases = @('-AllowDirty') }
    'no-backup' = @{ Kind = 'Switch'; Aliases = @('-NoBackup') }
    'allow-directory-deletion' = @{ Kind = 'Switch'; Aliases = @('--allow-directory-delete', '-AllowDirectoryDeletion') }
}

if ($cli['help']) {
    Write-CliHelp @"
Usage: .\scripts\apply-changes.ps1 [options]

Options:
  --zip <path>                  Apply a specific changed-files ZIP.
  -p, --project <path>          Target project root.
  --downloads <path>            Directory used for automatic ZIP discovery.
  --pattern <glob>              ZIP discovery pattern. Default: apollo-*-changed-files*.zip
  --strip-root                  Strip one redundant top-level archive directory.
  -n, --dry-run                 Preview without modifying files.
  --allow-dirty                 Allow tracked working-tree changes.
  --no-backup                   Skip recovery backups (including new paths).
  --allow-directory-deletion    Permit dir: entries in the deletion manifest.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
"@
    return
}

# Adapted from Archeion 4b711fc2 (GPL-3.0); retain Apollo's strict ZIP preflight.
$deleteManifestName = '.chatgpt-delete-manifest.txt'
$packageManifestName = '.apollo-change-package.json'
if (Test-RepositoryFileSystemRoot -Path $cli['project']) { throw 'Transfer cannot target a filesystem root.' }
$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
Assert-TransferRoot -Path $projectRoot
$zipPath = $cli['zip']
$downloadsPath = [string]$cli['downloads']
$zipPattern = if ($cli['pattern']) { [string]$cli['pattern'] } else { 'apollo-*-changed-files*.zip' }
$stripRoot = [bool]$cli['strip-root']
$dryRun = [bool]$cli['dry-run']
$allowDirty = [bool]$cli['allow-dirty']
$noBackup = [bool]$cli['no-backup']
$allowDirectoryDeletion = [bool]$cli['allow-directory-deletion']
$comparison = Get-RepositoryPathComparison

function Read-PackageManifest {
    param([string]$Path)
    $document = $null
    try {
        $document = [System.Text.Json.JsonDocument]::Parse([IO.File]::ReadAllText($Path), [System.Text.Json.JsonDocumentOptions]::new())
        $root = $document.RootElement
        if ($root.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) { throw 'Package manifest must be an object.' }
        $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($property in $root.EnumerateObject()) {
            if (-not $names.Add($property.Name)) { throw "Duplicate package manifest property: $($property.Name)" }
        }
        if ($names.Count -ne 8) { throw 'Unexpected package provenance fields.' }
        if ($root.GetProperty('schemaVersion').GetInt32() -ne 1) { throw 'Unsupported package provenance schema version.' }
        if ($root.GetProperty('packageKind').GetString() -cne 'apollo-change-package' -or $root.GetProperty('projectName').GetString() -cne 'Apollo') { throw 'Wrong-project package provenance.' }
        $commit = $root.GetProperty('sourceCommit').GetString()
        if ($commit -notmatch '^(?:[0-9a-f]{40}|[0-9a-f]{64})$') { throw 'Invalid source commit ID.' }
        $tracked = $root.GetProperty('trackedOnly').GetBoolean()
        $included = $root.GetProperty('includedFiles').GetInt64()
        $deleted = $root.GetProperty('deletedPaths').GetInt64()
        if ($included -lt 0 -or $deleted -lt 0) { throw 'Package counts must be nonnegative integers.' }
        $created = $root.GetProperty('createdAt')
        if ($created.GetString() -notmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,7})?(?:Z|\+00:00)$' -or $created.GetDateTimeOffset().Offset -ne [TimeSpan]::Zero) { throw 'Package timestamp must be UTC ISO-8601.' }
        return [pscustomobject]@{ sourceCommit = $commit; includedFiles = $included; deletedPaths = $deleted; trackedOnly = $tracked }
    } catch { throw "Package provenance manifest is malformed: $($_.Exception.Message)" }
    finally { if ($null -ne $document) { $document.Dispose() } }
}

function Get-ProjectMatchScore {
    param([Parameter(Mandatory)][string]$CandidateRoot)

    $score = 0
    $files = @(Get-ChildItem -LiteralPath $CandidateRoot -File -Recurse -Force |
        Where-Object { -not (Test-TransferMetadata -Path $_.Name) } |
        Select-Object -First 500)
    foreach ($file in $files) {
        $relativePath = [System.IO.Path]::GetRelativePath($CandidateRoot, $file.FullName)
        try {
            $targetPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'import'
        }
        catch {
            continue
        }
        if (Test-Path -LiteralPath $targetPath) {
            $score++
        }
    }
    return $score
}

function New-TransferBackup {
    param([string]$ProjectRoot)
    $base = Join-Path (Get-ApolloImportBase) (Get-RepositorySlug -ProjectRoot $ProjectRoot)
    $backup = Join-Path $base ((Get-Date -Format 'yyyyMMdd-HHmmssfffffff') + '-' + [Guid]::NewGuid().ToString('N'))
    Assert-TransferUnlinked -Path $backup
    if ($backup.StartsWith("$ProjectRoot$([IO.Path]::DirectorySeparatorChar)", (Get-RepositoryPathComparison))) { throw 'Recovery backup must be outside the project.' }
    [void][IO.Directory]::CreateDirectory((Join-Path $backup 'journal'))
    [void][IO.Directory]::CreateDirectory((Join-Path $backup 'originals'))
    [void][IO.Directory]::CreateDirectory((Join-Path $backup 'directory-receipts'))
    Write-TransferJson -Path (Join-Path $backup '.apollo-import-backup.json') -Value @{ schemaVersion = 1; projectRoot = $ProjectRoot }
    return $backup
}

function Add-TransferBackupRecord {
    param([string]$BackupRoot, [string]$ProjectRoot, $Item, [string]$Before, [string]$After, [int]$Sequence)
    $original = Join-Path (Join-Path $BackupRoot 'originals') $Item.RelativePath
    $createdDirectories = [Collections.Generic.List[string]]::new()
    if ($Before -eq 'absent' -and $After -ne 'absent') {
        for ($parent = Split-Path $Item.TargetPath -Parent; -not (Test-Path -LiteralPath $parent); $parent = Split-Path $parent -Parent) {
            $createdDirectories.Add([IO.Path]::GetRelativePath($ProjectRoot, $parent).Replace('\', '/'))
        }
    }
    $originalDigest = $null
    if ($Before -ne 'absent') {
        [void][IO.Directory]::CreateDirectory((Split-Path $original -Parent))
        if ($Before -eq 'directory') {
            $expected = Get-TransferTree -Path $Item.TargetPath
            $originalDigest = Get-TransferTreeDigest -Tree $expected
            Copy-Item -LiteralPath $Item.TargetPath -Destination $original -Recurse -Force
            $actual = Get-TransferTree -Path $original
            if ($actual.Count -ne $expected.Count -or -not (Test-TransferTreeSubset -Actual $actual -Expected $expected)) { throw 'Directory backup verification failed.' }
        } else {
            Copy-Item -LiteralPath $Item.TargetPath -Destination $original -Force
            if ((Get-TransferFileState -Path $original) -cne $Before) { throw 'File backup verification failed.' }
        }
    }
    # The record is published only after the original is safely copied, and
    # before any target mutation. A failed write leaves the previous records intact.
    Write-TransferJson -Path (Join-Path (Join-Path $BackupRoot 'journal') ('{0:D8}.json' -f $Sequence)) -Value @{
        path = $Item.RelativePath; before = $Before; after = $After; createdDirectories = @($createdDirectories); originalDigest = $originalDigest
    }
    return @($createdDirectories)
}

function New-ImportParents {
    param([string[]]$Directories, [string]$BackupRoot, [int]$Sequence)
    $index = 0
    foreach ($relative in ($Directories | Sort-Object Length)) {
        $directory = Resolve-TransferPath -Root $projectRoot -Path $relative
        if (Test-Path -LiteralPath $directory) { throw "Parent directory appeared during import: $relative" }
        [void][IO.Directory]::CreateDirectory($directory)
        $receipt = Join-Path (Join-Path $BackupRoot 'directory-receipts') ('{0:D8}-{1:D4}.json' -f $Sequence, (++$index))
        Write-TransferJson -Path $receipt -Value @{ path = $relative; creationTicks = [IO.Directory]::GetCreationTimeUtc($directory).Ticks }
    }
}

$gitProbe = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--is-inside-work-tree') -AllowFailure
$targetIsGit = $gitProbe.ExitCode -eq 0 -and $gitProbe.StdOut.Trim() -eq 'true'
if (-not $zipPath) {
    if (-not (Test-Path -LiteralPath $downloadsPath -PathType Container)) { throw "Downloads directory does not exist: $downloadsPath" }
    $latestZip = Get-ChildItem -LiteralPath $downloadsPath -File -Filter $zipPattern | Where-Object {
        $_.Name -match '^apollo-[a-z0-9][a-z0-9._-]*-changed-files(?: \(\d+\))?\.zip$' -and $_.Name -notmatch 'offline-bundle'
    } | Sort-Object LastWriteTime, Name -Descending | Select-Object -First 1
    if ($null -eq $latestZip) { throw "No changed-files ZIP matching '$zipPattern' was found in $downloadsPath" }
    $zipPath = $latestZip.FullName
}
$zipPath = [IO.Path]::GetFullPath([string]$zipPath)
if (-not (Test-Path -LiteralPath $zipPath -PathType Leaf)) { throw "ZIP file does not exist: $zipPath" }
if ([IO.Path]::GetExtension($zipPath) -ine '.zip') { throw "Expected a .zip file: $zipPath" }
Assert-TransferUnlinked -Path $zipPath
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ('apollo-import-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($tempRoot)
$backupRoot = $null
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $archivePaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        $filePaths = [Collections.Generic.List[string]]::new()
        $directoryPaths = [Collections.Generic.List[string]]::new()
        $spellings = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $archive.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            $isDirectory = $name.EndsWith('/')
            $path = ConvertTo-TransferPath -Path $(if ($isDirectory) { $name.Substring(0, $name.Length - 1) } else { $name })
            if (-not $archivePaths.Add($path)) { throw "ZIP contains duplicate entry path: $path" }
            $type = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ($type -notin @(0, 0x8000, 0x4000) -or ($entry.ExternalAttributes -band 0x400) -or ($type -eq 0x4000 -and -not $isDirectory) -or ($type -eq 0x8000 -and $isDirectory)) { throw "ZIP contains a linked or unsupported entry: $path" }
            if ($path -match '(?:^|/)EXPORT_MANIFEST\.json(?:/|$)') { throw 'This is a snapshot ZIP. Extract it separately instead of applying it as a changed-files package.' }
            if (Test-TransferMetadata -Path $path) {
                if ($isDirectory -or (Split-Path $path -Leaf) -notin @($deleteManifestName, $packageManifestName)) { throw "Unexpected archive metadata: $path" }
            }
            for ($current = $path; $current; $current = $current -replace '/[^/]+$','') {
                if ($spellings.ContainsKey($current) -and $spellings[$current] -cne $current) { throw "Case-colliding ZIP entry: $path" }
                $spellings[$current] = $current
                if (-not $current.Contains('/')) { break }
            }
            if ($isDirectory) { $directoryPaths.Add($path) } else { $filePaths.Add($path) }
        }
        Assert-TransferDisjoint -Paths @($filePaths)
        $fileSet = [Collections.Generic.HashSet[string]]::new([string[]]@($filePaths), [StringComparer]::OrdinalIgnoreCase)
        foreach ($path in $directoryPaths) {
            for ($current = $path; $current; $current = $current -replace '/[^/]+$','') {
                if ($fileSet.Contains($current)) { throw "Conflicting ZIP directory and file: $path" }
                if (-not $current.Contains('/')) { break }
            }
        }
        foreach ($entry in $archive.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            $destination = Join-Path $tempRoot $name.TrimEnd('/')
            if ($name.EndsWith('/')) { [void][IO.Directory]::CreateDirectory($destination); continue }
            [void][IO.Directory]::CreateDirectory((Split-Path $destination -Parent))
            $inputStream = $entry.Open()
            try {
                $outputStream = [IO.File]::Open($destination, [IO.FileMode]::CreateNew)
                try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose() }
            } finally { $inputStream.Dispose() }
        }
    } finally { $archive.Dispose() }

    $allFiles = @(Get-ChildItem -LiteralPath $tempRoot -File -Recurse -Force)
    $packageCandidates = @($allFiles | Where-Object { $_.Name -ieq $packageManifestName })
    $deleteCandidates = @($allFiles | Where-Object { $_.Name -ieq $deleteManifestName })
    if ($packageCandidates.Count -gt 1 -or $deleteCandidates.Count -gt 1) { throw 'ZIP contains duplicate package metadata.' }
    $packageManifest = if ($packageCandidates.Count) { Read-PackageManifest -Path $packageCandidates[0].FullName } else { $null }
    $contentRoot = $tempRoot
    $topLevelContent = @(Get-ChildItem -LiteralPath $tempRoot -Force | Where-Object { -not (Test-TransferMetadata -Path $_.Name) })
    if ($topLevelContent.Count -eq 1 -and $topLevelContent[0].PSIsContainer) {
        $child = $topLevelContent[0].FullName
        $metadataInChild = @($packageCandidates + $deleteCandidates | Where-Object { $_.DirectoryName -eq $child }).Count -gt 0
        $canonical = $packageCandidates.Count -and $packageCandidates[0].DirectoryName -eq $tempRoot
        if ($stripRoot -or $metadataInChild -or (-not $canonical -and ((Get-ProjectMatchScore -CandidateRoot $child) -gt (Get-ProjectMatchScore -CandidateRoot $tempRoot) -or $topLevelContent[0].Name -ieq [IO.Path]::GetFileNameWithoutExtension($zipPath)))) { $contentRoot = $child }
    } elseif ($stripRoot) { throw '--strip-root requires one top-level directory.' }
    foreach ($metadata in @($packageCandidates + $deleteCandidates)) {
        if ($metadata.DirectoryName -notin @($tempRoot, $contentRoot)) { throw "Metadata is not at an archive root: $($metadata.FullName)" }
    }
    $files = @($allFiles | Where-Object { -not (Test-TransferMetadata -Path $_.Name) })
    $copyEntries = [Collections.Generic.List[object]]::new()
    $deletions = [Collections.Generic.List[object]]::new()
    foreach ($file in $files) {
        $relativePath = ConvertTo-TransferPath -Path ([IO.Path]::GetRelativePath($contentRoot, $file.FullName))
        $targetPath = Resolve-TransferPath -Root $projectRoot -Path $relativePath
        if ($targetPath.Equals($zipPath, $comparison)) { throw 'ZIP cannot overwrite itself.' }
        if (Test-Path -LiteralPath $targetPath -PathType Container) { throw "Refusing to replace a directory with a file: $relativePath" }
        $copyEntries.Add([pscustomobject]@{ RelativePath = $relativePath; SourcePath = $file.FullName; TargetPath = $targetPath; Directory = $false })
    }
    if ($deleteCandidates.Count) {
        foreach ($line in (Get-Content -LiteralPath $deleteCandidates[0].FullName)) {
            $entry = $line.Trim()
            if (-not $entry -or $entry.StartsWith('#')) { continue }
            $isDirectory = $entry.StartsWith('dir:', [StringComparison]::OrdinalIgnoreCase)
            $relativePath = ConvertTo-TransferPath -Path $(if ($isDirectory) { $entry.Substring(4).Trim() } else { $entry })
            $targetPath = Resolve-TransferPath -Root $projectRoot -Path $relativePath
            if ($zipPath.Equals($targetPath, $comparison) -or $zipPath.StartsWith("$targetPath$([IO.Path]::DirectorySeparatorChar)", $comparison)) { throw 'ZIP cannot delete itself.' }
            if ($isDirectory -and -not $allowDirectoryDeletion) { throw "Directory deletion requested for '$relativePath'. Rerun with --allow-directory-deletion after reviewing the manifest." }
            if (Test-Path -LiteralPath $targetPath) {
                if ((Test-Path -LiteralPath $targetPath -PathType Container) -ne $isDirectory) { throw "Deletion manifest path type does not match: $relativePath (directories require dir:)" }
                if ($isDirectory) {
                    $tree = Get-TransferTree -Path $targetPath
                    foreach ($key in $tree.Keys) {
                        if ($key -ne '.') { [void](Resolve-TransferPath -Root $projectRoot -Path "$relativePath/$key") }
                    }
                }
            }
            $deletions.Add([pscustomobject]@{ RelativePath = $relativePath; TargetPath = $targetPath; Directory = $isDirectory })
        }
    }
    Assert-TransferDisjoint -Paths @(@($copyEntries | ForEach-Object RelativePath) + @($deletions | ForEach-Object RelativePath))
    if (-not $copyEntries.Count -and -not $deletions.Count) { throw 'The ZIP does not contain any files or deletion manifest to apply.' }
    if ($null -ne $packageManifest -and ($packageManifest.includedFiles -ne $copyEntries.Count -or $packageManifest.deletedPaths -ne $deletions.Count)) { throw 'Package provenance counts do not match archive contents.' }

    if ($targetIsGit -and -not $allowDirty -and (Test-GitTrackedDirty -ProjectRoot $projectRoot)) {
        throw 'The Git working tree has tracked changes. Commit, stash, or rerun with --allow-dirty after reviewing the risk.'
    }

    Write-CliHeading -Text $(if ($dryRun) { 'Changed-files preview' } else { 'Changed-files apply' })
    Write-CliDetail -Label 'Archive' -Value $zipPath -ValueTone 'Important'
    Write-CliDetail -Label 'Project' -Value $projectRoot -ValueTone 'Important'
    if ($null -eq $packageManifest) { Write-CliWarning -Message 'Provenance unavailable (legacy package).' }
    else {
        Write-CliDetail -Label 'Package' -Value 'Apollo'
        Write-CliDetail -Label 'Source commit' -Value $packageManifest.sourceCommit
        if ($targetIsGit) {
            $probe = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('cat-file', '-e', "$($packageManifest.sourceCommit)^{commit}") -AllowFailure
            if ($probe.ExitCode -eq 0) { Write-CliDetail -Label 'Baseline' -Value 'source commit present in target' -ValueTone 'Success' }
            else { Write-CliWarning -Message 'Source commit is not present in the target repository. Review the intended baseline.' }
        }
    }
    if ($allowDirty) { Write-CliWarning -Message '--allow-dirty is enabled. Imported paths may overlap local changes.' }
    if ($noBackup -and -not $dryRun) { Write-CliWarning -Message '--no-backup is enabled. No recovery backup will be created.' }
    $copiedCount = 0; $unchangedCount = 0; $deletedCount = 0; $skippedCount = 0
    $sequence = 0
    foreach ($item in ($copyEntries | Sort-Object RelativePath)) {
        $after = Get-TransferFileState -Path $item.SourcePath
        $before = Get-TransferFileState -Path $item.TargetPath
        if ($before -ceq $after) { $unchangedCount++; Write-CliStatus -Label 'UNCHANGED' -Message $item.RelativePath -Tone 'Muted'; continue }
        if ($dryRun) { $copiedCount++; Write-CliStatus -Label 'WOULD COPY' -Message $item.RelativePath -Tone 'Important'; continue }
        if (-not $noBackup) {
            if (-not $backupRoot) { $backupRoot = New-TransferBackup -ProjectRoot $projectRoot; Write-CliDetail -Label 'Backup' -Value $backupRoot -ValueTone 'Important' }
            $sequence++
            $parents = @(Add-TransferBackupRecord -BackupRoot $backupRoot -ProjectRoot $projectRoot -Item $item -Before $before -After $after -Sequence $sequence)
            New-ImportParents -Directories $parents -BackupRoot $backupRoot -Sequence $sequence
        } else { [void][IO.Directory]::CreateDirectory((Split-Path $item.TargetPath -Parent)) }
        Copy-TransferFile -Source $item.SourcePath -Target $item.TargetPath
        $copiedCount++; Write-CliStatus -Label 'COPY' -Message $item.RelativePath -Tone 'Important'
    }
    foreach ($item in ($deletions | Sort-Object RelativePath)) {
        $before = Get-TransferFileState -Path $item.TargetPath
        if ($before -eq 'absent') { $skippedCount++; Write-CliStatus -Label 'SKIP' -Message "$($item.RelativePath) (already absent)" -Tone 'Muted'; continue }
        if ($dryRun) { $deletedCount++; Write-CliStatus -Label 'WOULD DELETE' -Message $item.RelativePath -Tone 'Warning'; continue }
        if (-not $noBackup) {
            if (-not $backupRoot) { $backupRoot = New-TransferBackup -ProjectRoot $projectRoot; Write-CliDetail -Label 'Backup' -Value $backupRoot -ValueTone 'Important' }
            $sequence++
            $null = Add-TransferBackupRecord -BackupRoot $backupRoot -ProjectRoot $projectRoot -Item $item -Before $before -After 'absent' -Sequence $sequence
        }
        Remove-Item -LiteralPath $item.TargetPath -Recurse:$item.Directory -Force
        $deletedCount++; Write-CliStatus -Label 'DELETE' -Message $item.RelativePath -Tone 'Warning'
    }
    Write-CliText -Text ''
    Write-CliSuccess -Message $(if ($dryRun) { 'Preview complete' } elseif ($copiedCount -or $deletedCount) { 'Changes applied' } else { 'No changes needed' })
    Write-CliDetail -Label $(if ($dryRun) { 'Would copy' } else { 'Updated' }) -Value "$copiedCount files"
    Write-CliDetail -Label $(if ($dryRun) { 'Would delete' } else { 'Deleted' }) -Value "$deletedCount paths"
    Write-CliDetail -Label 'Skipped' -Value "$skippedCount paths"
    Write-CliDetail -Label 'Unchanged' -Value "$unchangedCount files"
    if ($dryRun) { Write-CliDetail -Label 'Mode' -Value 'dry run' }
    elseif ($backupRoot) { Write-CliDetail -Label 'Backup' -Value $backupRoot -ValueTone 'Important' }
    if (-not $dryRun -and $targetIsGit) { Write-CliHeading -Text 'Git status'; & git -C $projectRoot status --short }
} catch {
    if ($backupRoot) { Write-CliWarning -Message "Apply interrupted. Recovery backup: $backupRoot" }
    throw
} finally {
    if (Test-Path -LiteralPath $tempRoot) { Remove-Item -LiteralPath $tempRoot -Recurse -Force }
}
