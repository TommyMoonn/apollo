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
  --pattern <glob>              ZIP discovery pattern. Default: <project-slug>-*-changed-files.zip
                                Windows duplicate suffixes like (1), (2), or repeated copies such as (1)(1) are accepted.
  --strip-root                  Strip one redundant top-level archive directory.
  -n, --dry-run                 Preview without modifying files.
  --allow-dirty                 Allow tracked working-tree changes.
  --no-backup                   Skip backups of overwritten/deleted paths.
  --allow-directory-deletion    Permit dir: entries in the deletion manifest.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
"@
    return
}

$deleteManifestName = '.chatgpt-delete-manifest.txt'
$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
$projectSlug = Get-RepositorySlug -ProjectRoot $projectRoot
$zipPath = $cli['zip']
$downloadsPath = [string]$cli['downloads']
$zipPattern = [string]$cli['pattern']
if ([string]::IsNullOrWhiteSpace($zipPattern)) {
    $zipPattern = "$projectSlug-*-changed-files.zip"
}
$stripRoot = [bool]$cli['strip-root']
$dryRun = [bool]$cli['dry-run']
$allowDirty = [bool]$cli['allow-dirty']
$noBackup = [bool]$cli['no-backup']
$allowDirectoryDeletion = [bool]$cli['allow-directory-deletion']
$comparison = Get-RepositoryPathComparison

function Test-FileContentEqual {
    param(
        [Parameter(Mandatory)][string]$Left,
        [Parameter(Mandatory)][string]$Right
    )

    if (-not (Test-Path -LiteralPath $Right -PathType Leaf)) {
        return $false
    }

    $leftInfo = Get-Item -LiteralPath $Left
    $rightInfo = Get-Item -LiteralPath $Right
    if ($leftInfo.Length -ne $rightInfo.Length) {
        return $false
    }

    return (Get-FileHash -LiteralPath $Left -Algorithm SHA256).Hash -eq (Get-FileHash -LiteralPath $Right -Algorithm SHA256).Hash
}

function Resolve-ExtractionPath {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$EntryPath
    )

    $normalized = $EntryPath.Replace('\', '/')
    if ([string]::IsNullOrWhiteSpace($normalized)) {
        throw 'ZIP contains an empty entry path.'
    }
    if ($normalized.StartsWith('/') -or $normalized.StartsWith('\') -or $normalized -match '^[A-Za-z]:') {
        throw "ZIP contains an absolute path: $EntryPath"
    }
    if ($normalized -match '(^|/)\.\.(/|$)') {
        throw "ZIP contains parent-directory traversal: $EntryPath"
    }
    if ($normalized -match '(^|/)\.git(/|$)') {
        throw "ZIP contains forbidden .git content: $EntryPath"
    }

    $fullRoot = [System.IO.Path]::GetFullPath($Root).TrimEnd([char[]]@('\', '/'))
    $fullPath = [System.IO.Path]::GetFullPath((Join-Path $fullRoot $normalized))
    $prefix = "$fullRoot$([System.IO.Path]::DirectorySeparatorChar)"
    if (-not $fullPath.StartsWith($prefix, $comparison)) {
        throw "ZIP entry resolves outside its extraction root: $EntryPath"
    }

    return $fullPath
}

function Get-ProjectMatchScore {
    param([Parameter(Mandatory)][string]$CandidateRoot)

    $score = 0
    $files = @(Get-ChildItem -LiteralPath $CandidateRoot -File -Recurse -Force |
        Where-Object { $_.Name -ne $deleteManifestName } |
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

$script:backupRoot = $null
function Backup-ExistingPath {
    param(
        [Parameter(Mandatory)][string]$TargetPath,
        [Parameter(Mandatory)][string]$RelativePath
    )

    if ($noBackup -or -not (Test-Path -LiteralPath $TargetPath)) {
        return
    }

    if ($null -eq $script:backupRoot) {
        $timestamp = Get-Date -Format 'yyyyMMdd-HHmmssfff'
        $script:backupRoot = Join-Path (Join-Path (Get-ChatGptImportBase) $projectSlug) $timestamp
    }

    $backupPath = Join-Path $script:backupRoot $RelativePath
    $backupParent = Split-Path $backupPath -Parent
    New-Item -ItemType Directory -Path $backupParent -Force | Out-Null

    if (Test-Path -LiteralPath $TargetPath -PathType Container) {
        Copy-Item -LiteralPath $TargetPath -Destination $backupPath -Recurse -Force
    }
    else {
        Copy-Item -LiteralPath $TargetPath -Destination $backupPath -Force
    }
}

$gitProbe = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--is-inside-work-tree') -AllowFailure
if (-not $allowDirty -and $gitProbe.ExitCode -eq 0 -and $gitProbe.StdOut.Trim() -eq 'true' -and (Test-GitTrackedDirty -ProjectRoot $projectRoot)) {
    throw 'The Git working tree has tracked changes. Commit, stash, or rerun with --allow-dirty after reviewing the risk.'
}

if ([string]::IsNullOrWhiteSpace([string]$zipPath)) {
    if (-not (Test-Path -LiteralPath $downloadsPath -PathType Container)) {
        throw "Downloads directory does not exist: $downloadsPath"
    }

    $latestZip = Get-ChildItem -LiteralPath $downloadsPath -File -Filter '*.zip' |
        Where-Object {
            $discoveryName = $_.Name -replace '(?:\s*\([1-9][0-9]*\))+(?=\.zip$)', ''
            $_.Name -like $zipPattern -or $discoveryName -like $zipPattern
        } |
        Sort-Object LastWriteTime, Name -Descending |
        Select-Object -First 1
    if ($null -eq $latestZip) {
        throw "No ZIP matching '$zipPattern' was found in $downloadsPath"
    }
    $zipPath = $latestZip.FullName
}

$zipPath = [System.IO.Path]::GetFullPath([string]$zipPath)
if (-not (Test-Path -LiteralPath $zipPath -PathType Leaf)) {
    throw "ZIP file does not exist: $zipPath"
}
if (-not [System.IO.Path]::GetExtension($zipPath).Equals('.zip', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Expected a .zip file: $zipPath"
}

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("$projectSlug-import-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

try {
    Add-Type -AssemblyName System.IO.Compression
    $archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $archivePaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $archive.Entries) {
            $normalizedArchivePath = $entry.FullName.Replace('\', '/').TrimEnd('/')
            if (-not [string]::IsNullOrWhiteSpace($normalizedArchivePath) -and -not $archivePaths.Add($normalizedArchivePath)) {
                throw "ZIP contains duplicate entry path: $($entry.FullName)"
            }
        }

        foreach ($entry in $archive.Entries) {
            $entryName = $entry.FullName
            if ([string]::IsNullOrWhiteSpace($entryName)) {
                continue
            }

            $destination = Resolve-ExtractionPath -Root $tempRoot -EntryPath $entryName
            $isDirectory = $entryName.EndsWith('/') -or $entryName.EndsWith('\')
            if ($isDirectory) {
                New-Item -ItemType Directory -Path $destination -Force | Out-Null
                continue
            }

            $parent = Split-Path $destination -Parent
            New-Item -ItemType Directory -Path $parent -Force | Out-Null
            $inputStream = $entry.Open()
            try {
                $outputStream = [System.IO.File]::Open($destination, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
                try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose() }
            }
            finally { $inputStream.Dispose() }
        }
    }
    finally { $archive.Dispose() }

    $contentRoot = $tempRoot
    $topLevelContent = @(Get-ChildItem -LiteralPath $tempRoot -Force | Where-Object { $_.Name -ne $deleteManifestName })
    if ($topLevelContent.Count -eq 1 -and $topLevelContent[0].PSIsContainer) {
        $rootScore = Get-ProjectMatchScore -CandidateRoot $tempRoot
        $childScore = Get-ProjectMatchScore -CandidateRoot $topLevelContent[0].FullName
        $zipBaseName = [System.IO.Path]::GetFileNameWithoutExtension($zipPath)
        $rootMatchesArchive = $topLevelContent[0].Name.Equals($zipBaseName, [System.StringComparison]::OrdinalIgnoreCase)
        if ($stripRoot -or $childScore -gt $rootScore -or $rootMatchesArchive) {
            $contentRoot = $topLevelContent[0].FullName
        }
    }

    $files = @(Get-ChildItem -LiteralPath $contentRoot -File -Recurse -Force | Where-Object { $_.Name -ne $deleteManifestName })
    $manifestCandidates = @((Join-Path $contentRoot $deleteManifestName), (Join-Path $tempRoot $deleteManifestName)) | Select-Object -Unique
    $manifestPath = $manifestCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1

    if ($files.Count -eq 0 -and -not $manifestPath) {
        throw 'The ZIP does not contain any files or deletion manifest to apply.'
    }

    $copyEntries = [System.Collections.Generic.List[object]]::new()
    $copyRelativeSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $files) {
        $relativePath = [System.IO.Path]::GetRelativePath($contentRoot, $file.FullName).Replace('\', '/')
        if (-not $copyRelativeSet.Add($relativePath)) {
            throw "ZIP contains duplicate target path: $relativePath"
        }
        $targetPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'modify'
        $copyEntries.Add([pscustomobject]@{ RelativePath = $relativePath; SourcePath = $file.FullName; TargetPath = $targetPath })
    }

    $deletions = [System.Collections.Generic.List[object]]::new()
    $deleteRelativeSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    if ($manifestPath) {
        foreach ($line in (Get-Content -LiteralPath $manifestPath)) {
            $entry = $line.Trim()
            if (-not $entry -or $entry.StartsWith('#')) {
                continue
            }
            $isDirectory = $entry.StartsWith('dir:', [System.StringComparison]::OrdinalIgnoreCase)
            $relativePath = if ($isDirectory) { $entry.Substring(4).Trim() } else { $entry }
            if (-not $deleteRelativeSet.Add($relativePath)) {
                throw "Deletion manifest contains duplicate path: $relativePath"
            }
            if ($copyRelativeSet.Contains($relativePath)) {
                throw "ZIP both copies and deletes the same path: $relativePath"
            }
            $targetPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'delete'
            $deletions.Add([pscustomobject]@{ RelativePath = $relativePath; TargetPath = $targetPath; Directory = $isDirectory })
        }
    }

    # Validate every predictable target conflict before changing the working tree.
    # This keeps a bad manifest or file/directory collision from causing a partial apply.
    foreach ($item in $copyEntries) {
        if (Test-Path -LiteralPath $item.TargetPath -PathType Container) {
            throw "Refusing to replace a directory with a file: $($item.RelativePath)"
        }
    }

    foreach ($item in $deletions) {
        if (-not (Test-Path -LiteralPath $item.TargetPath)) {
            continue
        }

        $targetIsDirectory = Test-Path -LiteralPath $item.TargetPath -PathType Container
        if ($targetIsDirectory -and -not $item.Directory) {
            throw "Refusing to delete directory without an explicit 'dir:' prefix: $($item.RelativePath)"
        }
        if ($item.Directory -and -not $targetIsDirectory) {
            throw "Deletion manifest marks a file as a directory: $($item.RelativePath)"
        }
        if ($targetIsDirectory -and -not $allowDirectoryDeletion) {
            throw "Directory deletion requested for '$($item.RelativePath)'. Rerun with --allow-directory-deletion after reviewing the manifest."
        }
    }

    Write-Host "ZIP:     $zipPath"
    Write-Host "Project: $projectRoot"
    Write-Host "Source:  $contentRoot"
    Write-Host ''

    $copiedCount = 0
    $unchangedCount = 0
    $deletedCount = 0

    foreach ($item in ($copyEntries | Sort-Object RelativePath)) {
        if (Test-FileContentEqual -Left $item.SourcePath -Right $item.TargetPath) {
            $unchangedCount++
            Write-Host "UNCHANGED  $($item.RelativePath)"
            continue
        }
        if ($dryRun) {
            $copiedCount++
            Write-Host "WOULD COPY $($item.RelativePath)"
            continue
        }

        Backup-ExistingPath -TargetPath $item.TargetPath -RelativePath $item.RelativePath
        New-Item -ItemType Directory -Path (Split-Path $item.TargetPath -Parent) -Force | Out-Null
        Copy-Item -LiteralPath $item.SourcePath -Destination $item.TargetPath -Force
        $copiedCount++
        Write-Host "COPIED     $($item.RelativePath)"
    }

    foreach ($item in ($deletions | Sort-Object RelativePath)) {
        if (-not (Test-Path -LiteralPath $item.TargetPath)) {
            Write-Host "MISSING     $($item.RelativePath)"
            continue
        }

        $targetIsDirectory = Test-Path -LiteralPath $item.TargetPath -PathType Container

        if ($dryRun) {
            $deletedCount++
            Write-Host "WOULD DELETE $($item.RelativePath)"
            continue
        }

        Backup-ExistingPath -TargetPath $item.TargetPath -RelativePath $item.RelativePath
        Remove-Item -LiteralPath $item.TargetPath -Recurse:$targetIsDirectory -Force
        $deletedCount++
        Write-Host "DELETED    $($item.RelativePath)"
    }

    Write-Host ''
    Write-Host "Applied:   $copiedCount"
    Write-Host "Deleted:   $deletedCount"
    Write-Host "Unchanged: $unchangedCount"
    if ($dryRun) {
        Write-Host 'Mode:      dry run, no project files were changed'
    }
    elseif ($script:backupRoot) {
        Write-Host "Backup:    $script:backupRoot"
    }

    if (-not $dryRun -and $gitProbe.ExitCode -eq 0 -and $gitProbe.StdOut.Trim() -eq 'true') {
        Write-Host ''
        Write-Host 'Git status:'
        & git -C $projectRoot status --short
    }
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
