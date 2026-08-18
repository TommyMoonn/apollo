#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
    files = @{ Kind = 'Switch'; Aliases = @('-Detailed') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\review-changes.ps1 [options]

Options:
  -p, --project <path>          Project root to review.
  --files                       List every changed path.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
'@
    return
}

$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
$detailed = [bool]$cli['files']
Assert-GitRepository -ProjectRoot $projectRoot

function Get-ChangeCategory {
    param([Parameter(Mandatory)][string]$Path)

    $normalized = $Path.Replace('\', '/')
    $extension = [System.IO.Path]::GetExtension($normalized).ToLowerInvariant()
    $fileName = [System.IO.Path]::GetFileName($normalized)

    if ($normalized -match '(^|/)(tests?|fixtures)(/|$)' -or $normalized -match '\.(test|spec)\.[^/]+$') {
        return 'Tests'
    }
    if ($normalized.StartsWith('scripts/') -or $extension -eq '.ps1') {
        return 'Scripts'
    }
    if ($extension -eq '.rs') {
        return 'Rust'
    }
    if ($normalized.StartsWith('docs/') -or $extension -in @('.md', '.mdx', '.txt')) {
        return 'Documentation'
    }
    if ($normalized.StartsWith('.github/') -or $extension -in @('.toml', '.json', '.yaml', '.yml') -or $fileName -in @('.gitignore', '.zipignore')) {
        return 'Configuration'
    }
    if ($extension -in @('.png', '.jpg', '.jpeg', '.gif', '.svg', '.webp', '.ico', '.bmp')) {
        return 'Assets'
    }

    return 'Other'
}

$trackedChanges = @(Get-GitTrackedChanges -ProjectRoot $projectRoot)
$untrackedPaths = @(Get-GitUntrackedPaths -ProjectRoot $projectRoot)
$stagedPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$unstagedPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)

foreach ($path in (Get-GitNulValues -ProjectRoot $projectRoot -Arguments @('diff', '--cached', '--name-only', '-z', '--'))) {
    [void]$stagedPaths.Add($path)
}
foreach ($path in (Get-GitNulValues -ProjectRoot $projectRoot -Arguments @('diff', '--name-only', '-z', '--'))) {
    [void]$unstagedPaths.Add($path)
}

$allChanges = [System.Collections.Generic.List[object]]::new()
foreach ($change in $trackedChanges) {
    $allChanges.Add($change)
}
foreach ($path in $untrackedPaths) {
    $allChanges.Add([pscustomobject]@{ Status = '?'; Path = $path; OldPath = $null })
}

if ($allChanges.Count -eq 0) {
    Write-Host 'Working tree is clean.'
    return
}

$statusLabels = @{
    A = 'Added'; C = 'Copied'; D = 'Deleted'; M = 'Modified'; R = 'Renamed'; T = 'Type changed'; U = 'Unmerged'; '?' = 'Untracked'
}

Write-Host 'Change review'
Write-Host "Project: $projectRoot"
Write-Host ''
Write-Host 'Summary'
foreach ($status in @('A', 'M', 'D', 'R', 'C', 'T', 'U', '?')) {
    $count = @($allChanges | Where-Object Status -eq $status).Count
    if ($count -gt 0) {
        Write-Host ('  {0,-13} {1}' -f $statusLabels[$status], $count)
    }
}
Write-Host ('  {0,-13} {1}' -f 'Staged', $stagedPaths.Count)
Write-Host ('  {0,-13} {1}' -f 'Unstaged', $unstagedPaths.Count)
Write-Host ('  {0,-13} {1}' -f 'Total', $allChanges.Count)

Write-Host ''
Write-Host 'Areas'
foreach ($group in ($allChanges | Group-Object { Get-ChangeCategory -Path $_.Path } | Sort-Object Name)) {
    Write-Host ('  {0,-16} {1}' -f $group.Name, $group.Count)
}

$generatedPaths = @($allChanges | Where-Object {
    $_.Path.Replace('\', '/') -match '(^|/)(target|\.dev/chatgpt/bundles)(/|$)' -or $_.Path -match '\.(zip|tmp|log)$'
})
if ($generatedPaths.Count -gt 0) {
    Write-Host ''
    Write-Host 'Flags'
    Write-Host "  Generated or disposable output appears in the change set: $($generatedPaths.Path -join ', ')"
}

if ($detailed) {
    Write-Host ''
    Write-Host 'Files'
    foreach ($change in ($allChanges | Sort-Object Path, OldPath)) {
        $state = [System.Collections.Generic.List[string]]::new()
        if ($stagedPaths.Contains($change.Path) -or ($change.OldPath -and $stagedPaths.Contains($change.OldPath))) {
            $state.Add('staged')
        }
        if ($unstagedPaths.Contains($change.Path) -or ($change.OldPath -and $unstagedPaths.Contains($change.OldPath))) {
            $state.Add('unstaged')
        }
        if ($change.Status -eq '?') {
            $state.Add('untracked')
        }
        $stateLabel = if ($state.Count -gt 0) { ' [' + ($state -join ', ') + ']' } else { '' }
        if ($change.OldPath) {
            Write-Host "  $($change.Status) $($change.OldPath) -> $($change.Path)$stateLabel"
        }
        else {
            Write-Host "  $($change.Status) $($change.Path)$stateLabel"
        }
    }
}
