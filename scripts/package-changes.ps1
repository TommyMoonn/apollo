#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    name = @{ Aliases = @('-n', '-Name'); Default = 'changes' }
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
    output = @{ Aliases = @('-o', '-OutputPath') }
    'tracked-only' = @{ Kind = 'Switch'; Aliases = @('-ExcludeUntracked') }
    force = @{ Kind = 'Switch'; Aliases = @('-f', '-Force') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\package-changes.ps1 [options]

Options:
  -n, --name <slug>             Task or phase name used in the archive filename.
  -p, --project <path>          Project root to package.
  -o, --output <path>           Custom output ZIP path.
  --tracked-only                Exclude untracked files.
  -f, --force                   Replace an existing output ZIP.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
'@
    return
}

$deleteManifestName = '.chatgpt-delete-manifest.txt'
$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
$projectSlug = Get-RepositorySlug -ProjectRoot $projectRoot
$trackedOnly = [bool]$cli['tracked-only']
$force = [bool]$cli['force']
Assert-GitRepository -ProjectRoot $projectRoot

$cleanName = ([string]$cli['name']).Trim()
$cleanName = $cleanName -replace '\.zip$', ''
$cleanName = $cleanName -replace "^$([regex]::Escape($projectSlug))-", ''
$cleanName = $cleanName -replace '-changed-files$', ''
$cleanName = ($cleanName.ToLowerInvariant() -replace '[^a-z0-9._-]+', '-' -replace '-+', '-').Trim('-')
if ([string]::IsNullOrWhiteSpace($cleanName)) {
    $cleanName = 'changes'
}

$outputPath = $cli['output']
if ([string]::IsNullOrWhiteSpace([string]$outputPath)) {
    $outputPath = Join-Path (Split-Path $projectRoot -Parent) "$projectSlug-$cleanName-changed-files.zip"
}
else {
    $outputPath = [System.IO.Path]::GetFullPath([string]$outputPath)
    if (-not [System.IO.Path]::GetExtension($outputPath).Equals('.zip', [System.StringComparison]::OrdinalIgnoreCase)) {
        $outputPath = "$outputPath.zip"
    }
}

$comparison = Get-RepositoryPathComparison
$projectPrefix = "$projectRoot$([System.IO.Path]::DirectorySeparatorChar)"
if ($outputPath.StartsWith($projectPrefix, $comparison)) {
    $outputRelative = [System.IO.Path]::GetRelativePath($projectRoot, $outputPath)
    [void](Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $outputRelative -Operation 'write')
}

$copyPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
$deletePaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)

foreach ($change in (Get-GitTrackedChanges -ProjectRoot $projectRoot)) {
    if ($change.Status -in @('R', 'C')) {
        [void]$copyPaths.Add($change.Path)
        if ($change.Status -eq 'R') {
            [void]$deletePaths.Add($change.OldPath)
        }
        continue
    }

    if ($change.Status -eq 'D') {
        [void]$deletePaths.Add($change.Path)
    }
    else {
        [void]$copyPaths.Add($change.Path)
    }
}

if (-not $trackedOnly) {
    foreach ($path in (Get-GitUntrackedPaths -ProjectRoot $projectRoot)) {
        [void]$copyPaths.Add($path)
    }
}

foreach ($path in @($copyPaths)) {
    $sourcePath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $path -Operation 'package'
    if ($sourcePath.Equals($outputPath, $comparison) -or -not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        [void]$copyPaths.Remove($path)
    }
}

foreach ($path in @($deletePaths)) {
    $targetPath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $path -Operation 'package'
    if (Test-Path -LiteralPath $targetPath) {
        [void]$deletePaths.Remove($path)
    }
}

if ($copyPaths.Count -eq 0 -and $deletePaths.Count -eq 0) {
    throw 'No changed files were found.'
}

if (Test-Path -LiteralPath $outputPath) {
    if (-not $force) {
        throw "Output already exists: $outputPath. Use --force to replace it."
    }
    Remove-Item -LiteralPath $outputPath -Force
}

$outputParent = Split-Path $outputPath -Parent
New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
$stagingRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("$projectSlug-package-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stagingRoot -Force | Out-Null

try {
    foreach ($relativePath in ($copyPaths | Sort-Object)) {
        $sourcePath = Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativePath -Operation 'package'
        $destinationPath = Join-Path $stagingRoot $relativePath
        $destinationParent = Split-Path $destinationPath -Parent
        New-Item -ItemType Directory -Path $destinationParent -Force | Out-Null
        Copy-Item -LiteralPath $sourcePath -Destination $destinationPath -Force
    }

    if ($deletePaths.Count -gt 0) {
        [System.IO.File]::WriteAllLines(
            (Join-Path $stagingRoot $deleteManifestName),
            @($deletePaths | Sort-Object),
            [System.Text.UTF8Encoding]::new($false)
        )
    }

    Add-Type -AssemblyName System.IO.Compression
    $stream = [System.IO.File]::Open($outputPath, [System.IO.FileMode]::CreateNew)
    try {
        $archive = [System.IO.Compression.ZipArchive]::new($stream, [System.IO.Compression.ZipArchiveMode]::Create, $false)
        try {
            $files = @(Get-ChildItem -LiteralPath $stagingRoot -File -Recurse -Force | Sort-Object FullName)
            foreach ($file in $files) {
                $relativePath = [System.IO.Path]::GetRelativePath($stagingRoot, $file.FullName).Replace('\', '/')
                $entry = $archive.CreateEntry($relativePath, [System.IO.Compression.CompressionLevel]::Optimal)
                $entry.LastWriteTime = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
                $entryStream = $entry.Open()
                try {
                    $sourceStream = [System.IO.File]::OpenRead($file.FullName)
                    try { $sourceStream.CopyTo($entryStream) } finally { $sourceStream.Dispose() }
                }
                finally { $entryStream.Dispose() }
            }
        }
        finally { $archive.Dispose() }
    }
    finally { $stream.Dispose() }
}
finally {
    if (Test-Path -LiteralPath $stagingRoot) {
        Remove-Item -LiteralPath $stagingRoot -Recurse -Force
    }
}

Write-Host "Created:  $outputPath"
Write-Host "Included: $($copyPaths.Count) file(s)"
Write-Host "Deleted:  $($deletePaths.Count) path(s)"
if ($deletePaths.Count -gt 0) {
    Write-Host "Manifest: $deleteManifestName"
}
