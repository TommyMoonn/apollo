#requires -Version 7.0

# Optional, local-only smoke check. All writes are confined to disposable fixtures.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$pwsh = (Get-Process -Id $PID).Path
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert-Check {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Invoke-Tool {
    param([string]$Name, [string[]]$Arguments, [switch]$Reject)
    $output = @(& $pwsh -NoLogo -NoProfile -File (Join-Path $projectRoot "scripts/$Name.ps1") @Arguments 2>&1)
    $code = $LASTEXITCODE
    if (($Reject -and $code -eq 0) -or (-not $Reject -and $code -ne 0)) {
        throw "Unexpected exit code ($code) from ${Name}:`n$($output -join [Environment]::NewLine)"
    }
}

function Invoke-Git {
    param([string[]]$Arguments)
    & git @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Git failed: $($Arguments -join ' ')" }
}

function Read-ZipEntry {
    param([string]$Archive, [string]$Name)
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $entry = $zip.GetEntry($Name)
        if ($null -eq $entry) { return $null }
        $reader = [IO.StreamReader]::new($entry.Open())
        try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $zip.Dispose() }
}

# Parse scripts without executing build, release or publication commands.
$files = @(Get-ChildItem -LiteralPath (Join-Path $projectRoot 'scripts'), $PSScriptRoot -Filter '*.ps1' -File -Recurse)
foreach ($file in $files) {
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$tokens, [ref]$errors)
    if ($errors.Count -gt 0) {
        throw "PowerShell syntax error in $($file.FullName): $($errors -join '; ')"
    }
}
Write-Host "PASS PowerShell syntax ($($files.Count) files)"

$root = Join-Path ([IO.Path]::GetTempPath()) ('apollo-tooling-' + [Guid]::NewGuid().ToString('N'))
$source = Join-Path $root 'source repo'
$target = Join-Path $root 'target repo'
$originalAppData = $env:LOCALAPPDATA
try {
    New-Item -ItemType Directory -Path $source -Force | Out-Null
    $env:LOCALAPPDATA = Join-Path $root 'appdata'
    Invoke-Git @('-C', $source, 'init', '--quiet', '--template=')
    Invoke-Git @('-C', $source, 'config', 'core.excludesFile', (Join-Path $root 'empty-ignore'))
    Invoke-Git @('-C', $source, 'config', 'core.autocrlf', 'false')
    [IO.File]::WriteAllText((Join-Path $root 'empty-ignore'), '')
    Copy-Item -LiteralPath (Join-Path $projectRoot '.gitignore'), (Join-Path $projectRoot '.zipignore') -Destination $source
    Set-Content -LiteralPath (Join-Path $source 'seed.txt') -Value 'original' -NoNewline
    Set-Content -LiteralPath (Join-Path $source 'delete.txt') -Value 'deleted later' -NoNewline
    Invoke-Git @('-C', $source, 'add', '.')
    Invoke-Git @('-C', $source, '-c', 'user.name=Apollo Smoke', '-c', 'user.email=apollo@example.invalid', '-c', 'commit.gpgsign=false', '-c', "core.hooksPath=$root/no-hooks", 'commit', '--quiet', '-m', 'fixture')
    Invoke-Git @('clone', '--quiet', '--', $source, $target)

    Set-Content -LiteralPath (Join-Path $source 'seed.txt') -Value 'updated' -NoNewline
    Set-Content -LiteralPath (Join-Path $source 'new.txt') -Value 'new' -NoNewline
    Remove-Item -LiteralPath (Join-Path $source 'delete.txt')
    New-Item -ItemType Directory -Path (Join-Path $source 'target') | Out-Null
    Set-Content -LiteralPath (Join-Path $source 'target/output.txt') -Value 'generated' -NoNewline

    $repoZip = Join-Path $root 'repo.zip'
    $workspaceZip = Join-Path $root 'workspace.zip'
    Invoke-Tool 'zip-project' @('--project', $source, '--mode', 'repo', '--output', $repoZip)
    Invoke-Tool 'zip-project' @('--project', $source, '--mode', 'workspace', '--output', $workspaceZip)
    Assert-Check ((Read-ZipEntry $repoZip 'seed.txt') -ceq 'original') 'Repo export did not use committed content.'
    Assert-Check ((Read-ZipEntry $workspaceZip 'seed.txt') -ceq 'updated') 'Workspace export did not use current content.'
    Assert-Check ($null -eq (Read-ZipEntry $workspaceZip 'target/output.txt')) 'Workspace export included generated output.'
    Invoke-Tool 'review-changes' @('--project', $source, '--files')
    Write-Host 'PASS workspace export and review'

    $package = Join-Path $root 'apollo-smoke-changed-files.zip'
    Invoke-Tool 'package-changes' @('--project', $source, '--output', $package)
    Invoke-Tool 'apply-changes' @('--project', $target, '--zip', $package, '--dry-run')
    Assert-Check ((Get-Content -LiteralPath (Join-Path $target 'seed.txt') -Raw) -ceq 'original') 'Apply dry run changed a file.'
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $target 'new.txt'))) 'Apply dry run created a file.'
    Invoke-Tool 'apply-changes' @('--project', $target, '--zip', $package)
    Assert-Check ((Get-Content -LiteralPath (Join-Path $target 'seed.txt') -Raw) -ceq 'updated') 'Apply did not replace the file.'
    Assert-Check ((Get-Content -LiteralPath (Join-Path $target 'new.txt') -Raw) -ceq 'new') 'Apply did not add the file.'
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $target 'delete.txt'))) 'Apply did not delete the file.'
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $target '.apollo-change-package.json'))) 'Apply installed package metadata.'
    Set-Content -LiteralPath (Join-Path $target 'seed.txt') -Value 'later edit' -NoNewline
    Invoke-Tool 'restore-changes' @('--project', $target) -Reject
    Assert-Check ((Get-Content -LiteralPath (Join-Path $target 'seed.txt') -Raw) -ceq 'later edit') 'Restore overwrote a later edit.'
    Assert-Check (Test-Path -LiteralPath (Join-Path $target 'new.txt')) 'Restore removed a file despite a conflict.'
    Set-Content -LiteralPath (Join-Path $target 'seed.txt') -Value 'updated' -NoNewline
    Invoke-Tool 'restore-changes' @('--project', $target)
    Assert-Check ((Get-Content -LiteralPath (Join-Path $target 'seed.txt') -Raw) -ceq 'original') 'Restore did not recover original content.'
    Assert-Check ((Get-Content -LiteralPath (Join-Path $target 'delete.txt') -Raw) -ceq 'deleted later') 'Restore did not recover deleted content.'
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $target 'new.txt'))) 'Restore did not remove an imported file.'
    Write-Host 'PASS changed-file round trip and conflict protection'

    $unsafe = Join-Path $root 'unsafe.zip'
    $zip = [IO.Compression.ZipFile]::Open($unsafe, [IO.Compression.ZipArchiveMode]::Create)
    try {
        [void]$zip.CreateEntry('safe.txt')
        [void]$zip.CreateEntry('../outside.txt')
    } finally { $zip.Dispose() }
    Invoke-Tool 'apply-changes' @('--project', $target, '--zip', $unsafe) -Reject
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $root 'outside.txt'))) 'Apply wrote outside the repository.'
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $target 'safe.txt'))) 'Apply modified the target before rejecting an unsafe archive.'
    Write-Host 'PASS unsafe archive rejection'

    Invoke-Tool 'clean-generated' @('--project', $source, '--dry-run')
    Assert-Check (Test-Path -LiteralPath (Join-Path $source 'target/output.txt')) 'Cleanup dry run removed output.'
    Invoke-Git @('-C', $source, 'add', '-f', 'target/output.txt')
    Invoke-Tool 'clean-generated' @('--project', $source) -Reject
    Assert-Check (Test-Path -LiteralPath (Join-Path $source 'target/output.txt')) 'Cleanup removed tracked content.'
    Invoke-Git @('-C', $source, 'rm', '--cached', '-f', 'target/output.txt')
    Invoke-Tool 'clean-generated' @('--project', $source)
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $source 'target'))) 'Cleanup did not remove target.'
    Assert-Check ((Get-Content -LiteralPath (Join-Path $source 'seed.txt') -Raw) -ceq 'updated') 'Cleanup changed source content.'
    Write-Host 'PASS cleanup preview and tracked-file protection'
} finally {
    $env:LOCALAPPDATA = $originalAppData
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}

Write-Host 'Tooling smoke check passed.'
