#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
    backup = @{ Aliases = @('-BackupPath') }
    'dry-run' = @{ Kind = 'Switch'; Aliases = @('-n', '-DryRun') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\restore-changes.ps1 [options]

Options:
  -p, --project <path>          Project root to restore into.
  --backup <path>               Restore a specific import backup.
  -n, --dry-run                 Preview without restoring files.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
'@
    return
}

$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
$projectSlug = Get-RepositorySlug -ProjectRoot $projectRoot
$backupPath = $cli['backup']
$dryRun = [bool]$cli['dry-run']
$backupBase = Join-Path (Get-ChatGptImportBase) $projectSlug

if ([string]::IsNullOrWhiteSpace([string]$backupPath)) {
    if (-not (Test-Path -LiteralPath $backupBase -PathType Container)) {
        throw "No ChatGPT import backups were found for '$projectSlug' under: $backupBase"
    }
    $latestBackup = Get-ChildItem -LiteralPath $backupBase -Directory | Sort-Object Name -Descending | Select-Object -First 1
    if ($null -eq $latestBackup) {
        throw "No ChatGPT import backups were found for '$projectSlug' under: $backupBase"
    }
    $backupPath = $latestBackup.FullName
}
else {
    $backupPath = [System.IO.Path]::GetFullPath([string]$backupPath)
}

if (-not (Test-Path -LiteralPath $backupPath -PathType Container)) {
    throw "Backup directory does not exist: $backupPath"
}

$backupFiles = @(Get-ChildItem -LiteralPath $backupPath -File -Recurse -Force)
$backupDirectories = @(Get-ChildItem -LiteralPath $backupPath -Directory -Recurse -Force | Sort-Object { $_.FullName.Length })
if ($backupFiles.Count -eq 0 -and $backupDirectories.Count -eq 0) {
    throw "Backup directory is empty: $backupPath"
}

Write-Host "Backup:  $backupPath"
Write-Host "Project: $projectRoot"
Write-Host ''

$createdDirectoryCount = 0
$restoredFileCount = 0
foreach ($directory in $backupDirectories) {
    $relativePath = [System.IO.Path]::GetRelativePath($backupPath, $directory.FullName)
    $targetPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'restore'
    if (Test-Path -LiteralPath $targetPath -PathType Container) {
        continue
    }
    if ($dryRun) {
        Write-Host "WOULD CREATE  $relativePath"
    }
    else {
        New-Item -ItemType Directory -Path $targetPath -Force | Out-Null
        Write-Host "CREATED       $relativePath"
    }
    $createdDirectoryCount++
}

foreach ($file in ($backupFiles | Sort-Object FullName)) {
    $relativePath = [System.IO.Path]::GetRelativePath($backupPath, $file.FullName)
    $targetPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'restore'
    if ($dryRun) {
        Write-Host "WOULD RESTORE $relativePath"
        $restoredFileCount++
        continue
    }
    New-Item -ItemType Directory -Path (Split-Path $targetPath -Parent) -Force | Out-Null
    Copy-Item -LiteralPath $file.FullName -Destination $targetPath -Force
    Write-Host "RESTORED      $relativePath"
    $restoredFileCount++
}

Write-Host ''
Write-Host "Restored files:      $restoredFileCount"
Write-Host "Created directories: $createdDirectoryCount"
if ($dryRun) {
    Write-Host 'Mode: dry run, no project files were changed'
}
else {
    Write-Warning 'Import backups contain overwritten and deleted paths only. Files that were newly created by the import are not automatically removed by restore-changes.ps1.'
    $gitProbe = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--is-inside-work-tree') -AllowFailure
    if ($gitProbe.ExitCode -eq 0 -and $gitProbe.StdOut.Trim() -eq 'true') {
        Write-Host ''
        Write-Host 'Git status:'
        & git -C $projectRoot status --short
    }
}
