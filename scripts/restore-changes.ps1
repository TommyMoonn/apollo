#requires -Version 7.0

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "Cli.Common.ps1")

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path }
    backup = @{ Aliases = @('-BackupPath') }
    'dry-run' = @{ Kind = 'Switch'; Aliases = @('-n', '-DryRun') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\restore-changes.ps1 [options]

Options:
  -p, --project <path>          Project root to restore into.
  --backup <path>               Restore a specific backup.
  -n, --dry-run                 Preview without restoring files.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
'@
    return
}

# Adapted from Archeion 4b711fc2 (GPL-3.0); extend restoration with a recovery journal.
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')
if (Test-RepositoryFileSystemRoot -Path $cli['project']) { throw 'Transfer cannot target a filesystem root.' }
$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
Assert-TransferRoot -Path $projectRoot
$projectSlug = Get-RepositorySlug -ProjectRoot $projectRoot
$backupPath = $cli['backup']
$dryRun = [bool]$cli['dry-run']
if (-not $backupPath) {
    $candidates = @(foreach ($base in @((Get-ApolloImportBase), (Get-ChatGptImportBase))) {
        $directory = Join-Path $base $projectSlug
        if (Test-Path -LiteralPath $directory -PathType Container) {
            Assert-TransferUnlinked -Path $directory
            Get-ChildItem -LiteralPath $directory -Directory -Force | Where-Object {
                # Do not discover a new-format backup for another checkout with the same name.
                $marker = Join-Path $_.FullName '.apollo-import-backup.json'
                if ($base -eq (Get-ApolloImportBase)) {
                    if (Test-Path -LiteralPath $marker -PathType Leaf) {
                        Assert-TransferUnlinked -Path $marker
                        $header = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
                        $header.projectRoot -is [string] -and $header.projectRoot.Equals($projectRoot, (Get-RepositoryPathComparison))
                    } else { $false }
                } else { $true }
            }
        }
    })
    $latest = $candidates | Sort-Object Name -Descending | Select-Object -First 1
    if ($null -eq $latest) { throw "No import backups were found for '$projectSlug'." }
    $backupPath = $latest.FullName
} else { $backupPath = [IO.Path]::GetFullPath([string]$backupPath) }
if (-not (Test-Path -LiteralPath $backupPath -PathType Container)) { throw "Backup directory does not exist: $backupPath" }
Assert-TransferRoot -Path $backupPath
$comparison = Get-RepositoryPathComparison
$separator = [IO.Path]::DirectorySeparatorChar
if ($backupPath.Equals($projectRoot, $comparison) -or $backupPath.StartsWith("$projectRoot$separator", $comparison) -or $projectRoot.StartsWith("$backupPath$separator", $comparison)) { throw 'Backup and project directories must not overlap.' }

Write-CliHeading -Text $(if ($dryRun) { 'Backup restore preview' } else { 'Restore changed-files backup' })
Write-CliDetail -Label 'Backup' -Value $backupPath -ValueTone 'Important'
Write-CliDetail -Label 'Project' -Value $projectRoot -ValueTone 'Important'
$marker = Join-Path $backupPath '.apollo-import-backup.json'
$operations = [Collections.Generic.List[object]]::new()
$createdDirectories = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$directoryReceipts = [Collections.Generic.Dictionary[string,long]]::new([StringComparer]::OrdinalIgnoreCase)
$completion = Join-Path $backupPath 'restored.json'
$newFormat = Test-Path -LiteralPath $marker
$restoreDirectories = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)

if ($newFormat) {
    Assert-TransferUnlinked -Path $marker
    $header = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
    if ($header.schemaVersion -ne 1 -or $header.projectRoot -isnot [string] -or -not $header.projectRoot.Equals($projectRoot, $comparison)) { throw 'Backup schema or target project does not match.' }
    if (Test-Path -LiteralPath $completion) {
        Assert-TransferUnlinked -Path $completion
        Write-CliSuccess -Message 'Backup already restored'
        return
    }
    $journal = Join-Path $backupPath 'journal'
    $originals = Join-Path $backupPath 'originals'
    Assert-TransferUnlinked -Path $journal
    Assert-TransferUnlinked -Path $originals
    if (-not (Test-Path -LiteralPath $journal -PathType Container) -or -not (Test-Path -LiteralPath $originals -PathType Container)) { throw 'Incomplete backup structure.' }
    $records = @(Get-ChildItem -LiteralPath $journal -File -Force -Filter '*.json' | Sort-Object Name)
    $paths = [Collections.Generic.List[string]]::new()
    foreach ($file in $records) {
        Assert-TransferUnlinked -Path $file.FullName
        $record = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
        $relative = ConvertTo-TransferPath -Path $record.path
        $paths.Add($relative)
        $target = Resolve-TransferPath -Root $projectRoot -Path $relative
        $original = Resolve-TransferPath -Root $originals -Path $relative
        $before = $record.before
        $after = $record.after
        if ($before -notmatch '^(?:absent|directory|file:[0-9A-F]{64})$' -or $after -notmatch '^(?:absent|file:[0-9A-F]{64})$' -or $before -eq $after -or ($before -eq 'directory' -and $after -ne 'absent')) { throw "Invalid backup operation: $relative" }
        foreach ($directory in @($record.createdDirectories)) {
            $directory = ConvertTo-TransferPath -Path $directory
            if ($before -ne 'absent' -or -not $relative.StartsWith("$directory/", [StringComparison]::Ordinal)) { throw "Invalid created-directory record: $directory" }
            [void]$createdDirectories.Add((Resolve-TransferPath -Root $projectRoot -Path $directory))
        }
        $current = Get-TransferFileState -Path $target
        if ($before -eq 'directory') {
            $expected = Get-TransferTree -Path $original
            if (-not $expected.ContainsKey('.') -or $expected['.'] -ne 'directory' -or (Get-TransferTreeDigest -Tree $expected) -cne $record.originalDigest) { throw "Missing or damaged directory backup: $relative" }
            $actual = Get-TransferTree -Path $target
            if (-not (Test-TransferTreeSubset -Actual $actual -Expected $expected)) { throw "Restore conflict: later edits or unrelated content at $relative" }
            # Recreate only missing entries. This also handles a partially completed deletion or restore.
            foreach ($key in $expected.Keys) {
                $childRelative = if ($key -eq '.') { $relative } else { "$relative/$key" }
                $childTarget = Resolve-TransferPath -Root $projectRoot -Path $childRelative
                if ($actual.ContainsKey($key)) { continue }
                if ($expected[$key] -eq 'directory') { [void]$restoreDirectories.Add($childTarget) }
                else {
                    $source = Resolve-TransferPath -Root $originals -Path $childRelative
                    $operations.Add([pscustomobject]@{ Path = $childRelative; Target = $childTarget; Source = $source; Before = 'absent'; Action = 'RESTORE' })
                }
            }
            continue
        }
        if ($before -ne 'absent' -and (Get-TransferFileState -Path $original) -cne $before) { throw "Missing or damaged file backup: $relative" }
        if ($current -ceq $before) { continue }
        if ($current -cne $after) { throw "Restore conflict: later edit at $relative" }
        if ($before -eq 'absent') { $operations.Add([pscustomobject]@{ Path = $relative; Target = $target; Source = $null; Before = $current; Action = 'REMOVE' }) }
        else { $operations.Add([pscustomobject]@{ Path = $relative; Target = $target; Source = $original; Before = $current; Action = 'RESTORE' }) }
    }
    Assert-TransferDisjoint -Paths @($paths)
    $receiptRoot = Join-Path $backupPath 'directory-receipts'
    Assert-TransferUnlinked -Path $receiptRoot
    if (-not (Test-Path -LiteralPath $receiptRoot -PathType Container)) { throw 'Incomplete backup directory receipts.' }
    foreach ($file in (Get-ChildItem -LiteralPath $receiptRoot -File -Force -Filter '*.json')) {
        Assert-TransferUnlinked -Path $file.FullName
        $receipt = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
        $directory = Resolve-TransferPath -Root $projectRoot -Path $receipt.path
        if (-not $createdDirectories.Contains($directory) -or $receipt.creationTicks -isnot [long] -or $receipt.creationTicks -le 0 -or $directoryReceipts.ContainsKey($directory)) { throw 'Invalid directory receipt.' }
        $directoryReceipts.Add($directory, $receipt.creationTicks)
    }
} else {
    if ((Test-Path -LiteralPath (Join-Path $backupPath 'journal')) -and (Test-Path -LiteralPath (Join-Path $backupPath 'originals'))) { throw 'Incomplete journal backup: missing recovery metadata.' }
    Write-CliWarning -Message 'Legacy backup: only overwritten/deleted paths are recoverable. Later edits cannot be detected and new files are not removed. Review before restoring.'
    $tree = Get-TransferTree -Path $backupPath
    if ($tree.Count -le 1) { throw "Backup directory is empty: $backupPath" }
    foreach ($relative in $tree.Keys) {
        if ($relative -eq '.') { continue }
        $target = Resolve-TransferPath -Root $projectRoot -Path $relative
        $current = Get-TransferFileState -Path $target
        if ($tree[$relative] -eq 'directory') {
            if ($current -notin @('absent','directory')) { throw "Restore conflict: expected directory at $relative" }
            if ($current -eq 'absent') { [void]$restoreDirectories.Add($target) }
        } else {
            if ($current -eq 'directory') { throw "Restore conflict: expected file at $relative" }
            if ($current -cne $tree[$relative]) { $operations.Add([pscustomobject]@{ Path = $relative; Target = $target; Source = (Join-Path $backupPath $relative); Before = $current; Action = 'RESTORE' }) }
        }
    }
}
# The full restore is preflighted before changing any target. Remove only empty
# directories recorded as import-created, deepest first, preserving other files.
foreach ($directory in $createdDirectories) {
    if ((Test-Path -LiteralPath $directory) -and -not (Test-Path -LiteralPath $directory -PathType Container)) { throw "Restore conflict: created directory became a file: $directory" }
}
$restored = 0; $removed = 0; $created = 0; $removedDirectories = 0
foreach ($directory in ($restoreDirectories | Sort-Object Length)) {
    if (-not $dryRun) { [void][IO.Directory]::CreateDirectory($directory) }
    Write-CliStatus -Label $(if ($dryRun) { 'WOULD CREATE' } else { 'CREATE' }) -Message ([IO.Path]::GetRelativePath($projectRoot, $directory)) -Tone 'Important'
    $created++
}
foreach ($operation in ($operations | Sort-Object Path)) {
    if (-not $dryRun) {
        Assert-TransferUnlinked -Path $operation.Target
        if ((Get-TransferFileState -Path $operation.Target) -cne $operation.Before) { throw "Restore conflict: target changed during restore: $($operation.Path)" }
        if ($operation.Action -eq 'REMOVE') { Remove-Item -LiteralPath $operation.Target -Force }
        else {
            [void][IO.Directory]::CreateDirectory((Split-Path $operation.Target -Parent))
            Copy-TransferFile -Source $operation.Source -Target $operation.Target
        }
    }
    Write-CliStatus -Label $(if ($dryRun) { "WOULD $($operation.Action)" } else { $operation.Action }) -Message $operation.Path -Tone $(if ($operation.Action -eq 'REMOVE') { 'Warning' } else { 'Important' })
    if ($operation.Action -eq 'REMOVE') { $removed++ } else { $restored++ }
}
# Predict empty directories in preview using the exact planned file/directory removals.
$removedPaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($operation in $operations) { if ($operation.Action -eq 'REMOVE') { [void]$removedPaths.Add($operation.Target) } }
foreach ($directory in ($createdDirectories | Sort-Object Length -Descending)) {
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) { continue }
    Assert-TransferUnlinked -Path $directory
    if (-not $directoryReceipts.ContainsKey($directory) -or ($IsWindows -and [IO.Directory]::GetCreationTimeUtc($directory).Ticks -ne $directoryReceipts[$directory])) {
        Write-CliWarning -Message "Preserving directory whose import creation cannot be confirmed: $directory"
        continue
    }
    $remaining = @(Get-ChildItem -LiteralPath $directory -Force | Where-Object { -not $dryRun -or -not $removedPaths.Contains($_.FullName) })
    if ($remaining.Count) { Write-CliWarning -Message "Preserving nonempty import-created directory: $directory"; continue }
    if (-not $dryRun) { [IO.Directory]::Delete($directory, $false) }
    [void]$removedPaths.Add($directory)
    Write-CliStatus -Label $(if ($dryRun) { 'WOULD REMOVE' } else { 'REMOVE' }) -Message ([IO.Path]::GetRelativePath($projectRoot, $directory)) -Tone 'Warning'
    $removedDirectories++
}
if ($newFormat -and -not $dryRun) { Write-TransferJson -Path $completion -Value @{ restoredAt = [DateTime]::UtcNow.ToString('o') } }
Write-CliSuccess -Message $(if ($dryRun) { 'Preview complete' } else { 'Restore complete' })
Write-CliDetail -Label 'Restored' -Value "$restored files"
Write-CliDetail -Label 'Removed' -Value "$removed import-created files"
Write-CliDetail -Label 'Created' -Value "$created directories"
Write-CliDetail -Label 'Removed directories' -Value "$removedDirectories directories"
if ($dryRun) { Write-CliDetail -Label 'Mode' -Value 'dry run' }
else {
    Write-CliDetail -Label 'Recovery backup retained' -Value $backupPath
    $gitProbe = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--is-inside-work-tree') -AllowFailure
    if ($gitProbe.ExitCode -eq 0 -and $gitProbe.StdOut.Trim() -eq 'true') { Write-CliHeading -Text 'Git status'; & git -C $projectRoot status --short }
}
