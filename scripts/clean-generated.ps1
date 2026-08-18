#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
    rust = @{ Kind = 'Switch'; Aliases = @('-Rust') }
    'chatgpt-bundles' = @{ Kind = 'Switch'; Aliases = @('-ChatGptBundles') }
    all = @{ Kind = 'Switch'; Aliases = @('-All') }
    'dry-run' = @{ Kind = 'Switch'; Aliases = @('-n', '-DryRun') }
    force = @{ Kind = 'Switch'; Aliases = @('-f', '-Force') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\clean-generated.ps1 [options]

Options:
  -p, --project <path>          Project root to clean.
  --rust                        Remove target/.
  --chatgpt-bundles             Remove .dev/chatgpt/bundles/.
  --all                         Enable all known generated targets.
  -n, --dry-run                 Preview cleanup only.
  -f, --force                   Permit cleanup when a selected target contains tracked files.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
'@
    return
}

$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
$rust = [bool]$cli['rust']
$chatGptBundles = [bool]$cli['chatgpt-bundles']
$all = [bool]$cli['all']
$dryRun = [bool]$cli['dry-run']
$force = [bool]$cli['force']

if ($all) {
    $rust = $true
    $chatGptBundles = $true
}
if (-not $rust -and -not $chatGptBundles) {
    Write-Host 'No cleanup target selected. Use --rust, --chatgpt-bundles, or --all.'
    return
}

function Get-PathSize {
    param([Parameter(Mandatory)][string]$Path)
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        return [long](Get-Item -LiteralPath $Path).Length
    }
    $measurement = Get-ChildItem -LiteralPath $Path -File -Recurse -Force -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum
    if ($null -eq $measurement.Sum) { return 0L }
    return [long]$measurement.Sum
}

function Format-ByteSize {
    param([Parameter(Mandatory)][long]$Bytes)
    if ($Bytes -ge 1GB) { return '{0:N2} GB' -f ($Bytes / 1GB) }
    if ($Bytes -ge 1MB) { return '{0:N2} MB' -f ($Bytes / 1MB) }
    if ($Bytes -ge 1KB) { return '{0:N2} KB' -f ($Bytes / 1KB) }
    return "$Bytes B"
}

function Get-TrackedPathsUnder {
    param([Parameter(Mandatory)][string]$RelativePath)

    $gitProbe = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--is-inside-work-tree') -AllowFailure
    if ($gitProbe.ExitCode -ne 0 -or $gitProbe.StdOut.Trim() -ne 'true') {
        return @()
    }

    $normalized = $RelativePath.Replace('\', '/')
    $result = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('ls-files', '--', $normalized, "$normalized/**")
    return @($result.StdOut -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

$targets = [System.Collections.Generic.List[string]]::new()
if ($rust) { $targets.Add('target') }
if ($chatGptBundles) { $targets.Add('.dev/chatgpt/bundles') }

$existingTargets = [System.Collections.Generic.List[object]]::new()
foreach ($relativePath in $targets) {
    $fullPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'remove'
    if (-not (Test-Path -LiteralPath $fullPath)) {
        continue
    }

    $tracked = @(Get-TrackedPathsUnder -RelativePath $relativePath)
    if ($tracked.Count -gt 0 -and -not $force) {
        throw "Refusing to remove '$relativePath' because it contains tracked files. Use --force only after reviewing: $($tracked -join ', ')"
    }

    $existingTargets.Add([pscustomobject]@{
        RelativePath = $relativePath
        FullPath = $fullPath
        Size = Get-PathSize -Path $fullPath
    })
}

if ($existingTargets.Count -eq 0) {
    Write-Host 'No selected generated output exists.'
    return
}

$totalBytes = 0L
foreach ($target in $existingTargets) {
    $totalBytes += $target.Size
    $sizeLabel = Format-ByteSize -Bytes $target.Size
    if ($dryRun) {
        Write-Host ('WOULD REMOVE {0,-32} {1,12}' -f $target.RelativePath, $sizeLabel)
    }
    else {
        Remove-Item -LiteralPath $target.FullPath -Recurse -Force
        Write-Host ('REMOVED      {0,-32} {1,12}' -f $target.RelativePath, $sizeLabel)
    }
}

Write-Host ''
if ($dryRun) {
    Write-Host "Would free approximately: $(Format-ByteSize -Bytes $totalBytes)"
    Write-Host 'Mode: dry run, nothing was removed'
}
else {
    Write-Host "Freed approximately: $(Format-ByteSize -Bytes $totalBytes)"
}
