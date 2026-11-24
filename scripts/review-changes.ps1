#requires -Version 7.0
# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
# Modified for Apollo on 2026-11-24.

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
  --files                       List each changed path and index/worktree state.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
'@
    return
}

$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
$detailed = [bool]$cli['files']
Assert-GitRepository -ProjectRoot $projectRoot
$gitRoot = (Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--show-toplevel')).StdOut.Trim()
if (-not [System.IO.Path]::GetFullPath($gitRoot).Equals($projectRoot, (Get-RepositoryPathComparison))) {
    throw 'Review requires the Git working tree root.'
}

function Get-ChangeCategory {
    param([Parameter(Mandatory)][string]$Path)

    $normalized = $Path.Replace('\', '/')
    $extension = [System.IO.Path]::GetExtension($normalized).ToLowerInvariant()
    $fileName = [System.IO.Path]::GetFileName($normalized)

    if ($normalized -match '(^|/)(tests?|fixtures)(/|$)' -or $normalized -match '\.(test|spec)\.[^/]+$') {
        return 'Tests'
    }
    if ($normalized.StartsWith('.github/workflows/')) { return 'CI' }
    if ($normalized.StartsWith('assets/')) { return 'Assets' }
    if ($normalized.StartsWith('scripts/') -or $extension -eq '.ps1') {
        return 'Scripts'
    }
    if ($extension -eq '.rs') {
        return 'Rust'
    }
    if ($normalized.StartsWith('docs/') -or $extension -in @('.md', '.mdx', '.txt')) {
        return 'Documentation'
    }
    if ($normalized.StartsWith('.github/') -or $extension -in @('.toml', '.json', '.yaml', '.yml') -or $fileName -in @('.gitignore', '.zipignore', 'justfile', 'Cargo.lock')) {
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

foreach ($path in (Get-GitNulValues -ProjectRoot $projectRoot -Arguments @('diff', '--cached', '--name-only', '--no-renames', '-z', '--'))) {
    [void]$stagedPaths.Add($path)
}
foreach ($path in (Get-GitNulValues -ProjectRoot $projectRoot -Arguments @('diff', '--name-only', '--no-renames', '-z', '--'))) {
    [void]$unstagedPaths.Add($path)
}

$allChanges = [System.Collections.Generic.List[object]]::new()
foreach ($change in $trackedChanges) {
    $allChanges.Add($change)
}
foreach ($path in $untrackedPaths) {
    $allChanges.Add([pscustomobject]@{ Status = '?'; Path = $path; OldPath = $null })
}

# HEAD-to-worktree is the exportable delta, but the index may contain a
# change that the worktree reverses. Report those paths without inventing a
# package operation for them (or incorrectly calling the tree clean).
$representedPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
foreach ($change in $allChanges) {
    [void]$representedPaths.Add($change.Path)
    if ($change.OldPath) { [void]$representedPaths.Add($change.OldPath) }
}
foreach ($path in (@($stagedPaths) + @($unstagedPaths) | Sort-Object -CaseSensitive -Unique)) {
    if ($representedPaths.Add($path)) {
        $allChanges.Add([pscustomobject]@{ Status = '~'; Path = $path; OldPath = $null })
    }
}

$statusLabels = @{
    A = 'Added'; C = 'Copied'; D = 'Deleted'; M = 'Modified'; R = 'Renamed'; T = 'Type changed'; U = 'Unmerged'; '?' = 'Untracked'; '~' = 'Index/worktree only'
}
function Test-ChangeState {
    param($Change, [System.Collections.Generic.HashSet[string]]$Paths)
    return ($Paths.Contains($Change.Path) -or ($Change.OldPath -and $Paths.Contains($Change.OldPath)))
}
$branch = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('symbolic-ref', '--quiet', '--short', 'HEAD') -AllowFailure
$branchName = if ($branch.ExitCode -eq 0) { $branch.StdOut.Trim() } elseif ($branch.ExitCode -eq 1) { '(detached HEAD)' } else { throw "Could not inspect branch: $($branch.StdErr)" }
Write-CliHeading -Text 'Change review'
Write-CliDetail -Label 'Project' -Value $projectRoot -ValueTone 'Important'
Write-CliDetail -Label 'Branch' -Value $branchName
Write-Host ''
Write-CliHeading -Text 'Summary'
foreach ($status in @('A', 'M', 'D', 'R', 'C', 'T', 'U', '?', '~')) {
    $count = @($allChanges | Where-Object Status -eq $status).Count
    if ($count -gt 0) { Write-CliDetail -Label $statusLabels[$status] -Value ([string]$count) }
}
Write-CliDetail -Label 'Staged' -Value ([string]@($allChanges | Where-Object { Test-ChangeState -Change $_ -Paths $stagedPaths }).Count)
Write-CliDetail -Label 'Unstaged' -Value ([string]@($allChanges | Where-Object { Test-ChangeState -Change $_ -Paths $unstagedPaths }).Count)
Write-CliDetail -Label 'Total' -Value ([string]$allChanges.Count)
if ($allChanges.Count -eq 0) {
    Write-Host ''
    Write-CliSuccess -Message 'Working tree is clean.'
    return
}

Write-Host ''
Write-CliHeading -Text 'Areas'
foreach ($group in ($allChanges | Group-Object { Get-ChangeCategory -Path $_.Path } | Sort-Object Name)) {
    Write-CliDetail -Label $group.Name -Value ([string]$group.Count)
}

$flags = [System.Collections.Generic.List[string]]::new()
$deleted = @($allChanges | Where-Object Status -eq 'D')
$sourceChanges = @($allChanges | Where-Object { (Get-ChangeCategory -Path $_.Path) -eq 'Rust' -and $_.Status -ne 'D' })
$testChanges = @($allChanges | Where-Object { (Get-ChangeCategory -Path $_.Path) -eq 'Tests' })
$generatedPaths = @($allChanges | Where-Object {
    @($_.Path, $_.OldPath) -match '^(target|artifacts|\.scratch|\.agent-work|\.dev/chatgpt/bundles|\.plans|\.planning|\.project|test-results|coverage)(/|$)|\.(zip|tmp|log|pem|key|pfx|p12)$|(^|/)(\.env(?:\..*)?|\.DS_Store|Thumbs\.db|Desktop\.ini)$'
})
$dependencyFiles = @($allChanges | Where-Object {
    [System.IO.Path]::GetFileName($_.Path) -in @('Cargo.toml', 'Cargo.lock') -or
    ($_.OldPath -and [System.IO.Path]::GetFileName($_.OldPath) -in @('Cargo.toml', 'Cargo.lock'))
})
$workflowFiles = @($allChanges | Where-Object { @($_.Path, $_.OldPath) -match '^\.github/workflows/' })
$largeFiles = @(
    foreach ($change in $allChanges | Where-Object Status -ne 'D') {
        $fullPath = Join-Path $projectRoot $change.Path
        if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
            $item = Get-Item -LiteralPath $fullPath -Force
            if ($item.Length -ge 1MB) { "$($change.Path) ($(Format-CliByteSize -Bytes $item.Length))" }
        }
    }
)
function Add-PathFlag {
    param([string]$Message, [AllowEmptyCollection()][string[]]$Paths)
    if ($Paths.Count -gt 0) {
        $detail = if ($detailed) { ': ' + ($Paths -join ', ') } else { " ($($Paths.Count) paths)" }
        $flags.Add($Message + $detail)
    }
}
if ($deleted.Count -gt 0) { $flags.Add("$($deleted.Count) deleted path(s). Confirm obsolete code and imports were removed cleanly.") }
if ($sourceChanges.Count -gt 0 -and $testChanges.Count -eq 0) {
    $flags.Add('Rust source changed without separate test file changes. Review inline Rust tests and relevant behavior coverage.')
}
Add-PathFlag -Message 'Generated, local or sensitive paths appear in the change set (review before packaging)' -Paths @($generatedPaths | ForEach-Object Path)
Add-PathFlag -Message 'Dependency manifests or lock files changed' -Paths @($dependencyFiles | ForEach-Object Path)
Add-PathFlag -Message 'GitHub workflow files changed' -Paths @($workflowFiles | ForEach-Object Path)
Add-PathFlag -Message 'Large changed files' -Paths $largeFiles
if (@($allChanges | Where-Object Status -eq '~').Count -gt 0) {
    $flags.Add('Index/worktree changes exist with no separate HEAD-to-worktree export operation.')
}
foreach ($check in @(@{ Label = 'Unstaged'; Arguments = @('diff', '--check', '--') }, @{ Label = 'Staged'; Arguments = @('diff', '--cached', '--check', '--') })) {
    $result = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments $check.Arguments -AllowFailure
    if ($result.ExitCode -eq 0) { continue }
    if ($result.ExitCode -ge 128 -or [string]::IsNullOrWhiteSpace($result.StdOut)) { throw "Git whitespace inspection failed: $($result.StdErr)" }
    $message = "$($check.Label) Git whitespace check failed"
    if ($detailed) { $message += ': ' + $result.StdOut.Trim() }
    $flags.Add($message)
}
Write-Host ''
Write-CliHeading -Text 'Review flags'
if ($flags.Count -eq 0) { Write-CliSuccess -Message 'No review flags detected' }
else { foreach ($flag in $flags) { Write-CliWarning -Message $flag } }

if ($detailed) {
    Write-Host ''
    Write-CliHeading -Text 'Files'
    foreach ($change in ($allChanges | Sort-Object Path, OldPath)) {
        $state = [System.Collections.Generic.List[string]]::new()
        if (Test-ChangeState -Change $change -Paths $stagedPaths) { $state.Add('staged') }
        if (Test-ChangeState -Change $change -Paths $unstagedPaths) { $state.Add('unstaged') }
        if ($change.Status -eq '?') { $state.Add('untracked') }
        $stateLabel = if ($state.Count -gt 0) { ' [' + ($state -join ', ') + ']' } else { '' }
        $path = if ($change.OldPath) { "$($change.OldPath) -> $($change.Path)" } else { $change.Path }
        Write-CliStatus -Label $statusLabels[$change.Status] -Message "$path$stateLabel" -MessageTone 'Important'
    }
} else {
    Write-Host ''
    Write-CliDetail -Label 'Files' -Value 'Use --files to list every changed path.' -ValueTone 'Muted'
}
