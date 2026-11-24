#requires -Version 7.0

# Adapted from Archeion scripts/clean-generated.ps1 at 4b711fc2 (GPL-3.0-only).
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
    rust = @{ Kind = 'Switch'; Aliases = @('-Rust') }
    'dry-run' = @{ Kind = 'Switch'; Aliases = @('-n', '-DryRun') }
    force = @{ Kind = 'Switch'; Aliases = @('-f', '-Force') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\clean-generated.ps1 [options]

Options:
  -p, --project <path>          Project root to clean.
  --rust                        Remove target/ (the default).
  -n, --dry-run                 Preview cleanup only.
  -f, --force                   Allow selected tracked content to be removed.
  -h, --help                    Show this help.

Legacy PowerShell aliases for these options remain accepted.
'@
    return
}

if (Test-RepositoryFileSystemRoot -Path $cli['project']) {
    throw 'Cleanup cannot target a filesystem root.'
}
$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
Assert-TransferUnlinked -Path $projectRoot
if ($projectRoot -match '(^|[\\/])(?:\.git|\.agent-work|\.dev|\.plans|\.planning|\.project|artifacts)([\\/]|$)') {
    throw "Cleanup cannot target a protected directory: $projectRoot"
}
$dryRun = [bool]$cli['dry-run']
$force = [bool]$cli['force']

# Refuse a subdirectory of a repository or a broken Git checkout. An extracted
# source snapshot may be cleaned without Git metadata if it has a Cargo manifest.
$gitRoot = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--show-toplevel') -AllowFailure
if ($gitRoot.ExitCode -eq 0) {
    $topLevel = [IO.Path]::GetFullPath($gitRoot.StdOut.TrimEnd([char[]]@("`r", "`n"))).TrimEnd([char[]]@('\', '/'))
    if (-not $topLevel.Equals($projectRoot, (Get-RepositoryPathComparison))) {
        throw "Cleanup requires the repository root: $topLevel"
    }
} elseif ((Test-Path -LiteralPath (Join-Path $projectRoot '.git')) -or -not (Test-Path -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -PathType Leaf)) {
    throw "Unable to establish a Git or Cargo project root: $projectRoot"
}

$modeLabel = if ($dryRun) { 'dry run' } else { 'apply' }
Write-CliHeading -Text 'Generated output cleanup'
Write-CliDetail -Label 'Project' -Value $projectRoot -ValueTone 'Important'
Write-CliDetail -Label 'Scope' -Value 'Rust target/'
Write-CliDetail -Label 'Mode' -Value $modeLabel
if ($force) {
    Write-CliWarning -Message 'Tracked-file protection is disabled by --force.'
}

$relativePath = 'target'
$fullPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'remove'
Assert-TransferUnlinked -Path $fullPath
Assert-TransferPathSpelling -Root $projectRoot -Path $fullPath

if (-not (Test-Path -LiteralPath $fullPath)) {
    Write-CliSuccess -Message 'No selected generated output exists.'
    return
}
if (-not (Test-Path -LiteralPath $fullPath -PathType Container)) {
    throw "Cleanup target must be a directory: $fullPath"
}

if ($gitRoot.ExitCode -eq 0) {
    $trackedPaths = @(Get-GitNulValues -ProjectRoot $projectRoot -Arguments @('ls-files', '-z', '--', $relativePath))
    if ($trackedPaths.Count -gt 0 -and -not $force) {
        throw "Refusing to remove '$relativePath' because it contains tracked files. Use --force only after reviewing: $($trackedPaths -join ', ')"
    }
}

# Measure and validate in one walk. Never recurse into a link, even with --force.
$totalBytes = 0L
$pending = [Collections.Generic.Stack[IO.DirectoryInfo]]::new()
$pending.Push([IO.DirectoryInfo]::new($fullPath))
while ($pending.Count -gt 0) {
    $directory = $pending.Pop()
    if ($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Symbolic links and reparse points are not supported: $($directory.FullName)"
    }
    foreach ($entry in $directory.EnumerateFileSystemInfos()) {
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Symbolic links and reparse points are not supported: $($entry.FullName)"
        }
        if ($entry.Name -match '^(?:\.git|\.agent-work|\.dev|\.plans|\.planning|\.project|artifacts)$|^.+-offline-bundle(?:\.part-\d+)?\.zip$') {
            throw "Protected local content inside cleanup target: $($entry.FullName)"
        }
        if ($entry -is [IO.DirectoryInfo]) {
            $pending.Push($entry)
        } else {
            $totalBytes += $entry.Length
        }
    }
}

$message = "$relativePath ($(Format-CliByteSize -Bytes $totalBytes))"
Write-CliText -Text ''
if ($dryRun) {
    Write-CliHeading -Text 'Cleanup preview'
    Write-CliStatus -Label 'WOULD REMOVE' -Message $message -Tone 'Important'
    Write-CliSuccess -Message 'Cleanup preview complete'
    Write-CliDetail -Label 'Would remove' -Value '1 target'
    Write-CliDetail -Label 'Would free' -Value (Format-CliByteSize -Bytes $totalBytes)
} else {
    Write-CliHeading -Text 'Cleanup'
    Assert-TransferUnlinked -Path $fullPath
    Remove-Item -LiteralPath $fullPath -Recurse -Force
    Write-CliStatus -Label 'REMOVE' -Message $message -Tone 'Important'
    Write-CliSuccess -Message 'Cleanup complete'
    Write-CliDetail -Label 'Removed' -Value '1 target'
    Write-CliDetail -Label 'Freed' -Value (Format-CliByteSize -Bytes $totalBytes)
}
