#requires -Version 7.0

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "Cli.Common.ps1")

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    name = @{ Aliases = @('-n', '-Name'); Default = 'changes' }
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path }
    output = @{ Aliases = @('-o', '-OutputPath') }
    'tracked-only' = @{ Kind = 'Switch'; Aliases = @('-ExcludeUntracked') }
    force = @{ Kind = 'Switch'; Aliases = @('-f', '-Force') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\package-changes.ps1 [options]

Options:
  -n, --name <slug>             Task name used in the archive filename.
  -p, --project <path>          Project root to package.
  -o, --output <path>           Custom output ZIP path.
  --tracked-only                Exclude untracked files.
  -f, --force                   Replace an existing output ZIP.
  -h, --help                    Show this help.

Legacy PowerShell flags remain accepted for compatibility.
'@
    return
}

# Adapted from Archeion 4b711fc2 (GPL-3.0); use Apollo's repository guards.
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$deleteManifestName = '.chatgpt-delete-manifest.txt'
$packageManifestName = '.apollo-change-package.json'
if (Test-RepositoryFileSystemRoot -Path $cli['project']) { throw 'Transfer cannot target a filesystem root.' }
$projectRoot = Resolve-RepositoryRoot -Path $cli['project']
Assert-TransferRoot -Path $projectRoot
Assert-GitRepository -ProjectRoot $projectRoot
$gitRoot = (Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--show-toplevel')).StdOut.Trim()
if (-not [IO.Path]::GetFullPath($gitRoot).Equals($projectRoot, (Get-RepositoryPathComparison))) { throw 'Package requires the Git repository root.' }
$sourceCommit = (Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('rev-parse', '--verify', 'HEAD^{commit}')).StdOut.Trim()
$trackedOnly = [bool]$cli['tracked-only']
$force = [bool]$cli['force']
$cleanName = ([string]$cli['name']).Trim().ToLowerInvariant()
if ($cleanName -notmatch '^[a-z0-9][a-z0-9._-]*$' -or $cleanName.EndsWith('.')) { throw 'Invalid package name slug.' }
$outputPath = if ($cli['output']) { [IO.Path]::GetFullPath([string]$cli['output']) } else { Join-Path (Split-Path $projectRoot -Parent) "apollo-$cleanName-changed-files.zip" }
if ([IO.Path]::GetExtension($outputPath) -ine '.zip') { $outputPath += '.zip' }
Assert-TransferUnlinked -Path $outputPath
$comparison = Get-RepositoryPathComparison
$projectPrefix = "$projectRoot$([IO.Path]::DirectorySeparatorChar)"
if ($outputPath.StartsWith($projectPrefix, $comparison)) {
    $relativeOutput = [IO.Path]::GetRelativePath($projectRoot, $outputPath).Replace('\', '/')
    [void](ConvertTo-TransferPath -Path $relativeOutput)
    [void](Resolve-RepositoryRelativePath -ProjectRoot $projectRoot -RelativePath $relativeOutput -Operation 'write')
    Assert-TransferPathSpelling -Root $projectRoot -Path $outputPath
    $trackedOutput = Invoke-RepositoryGit -ProjectRoot $projectRoot -Arguments @('ls-files', '--error-unmatch', '--', ":(literal)$relativeOutput") -AllowFailure
    if ($trackedOutput.ExitCode -eq 0) { throw 'Output overlaps a tracked project file.' }
}
Write-CliHeading -Text 'Changed-files package'
Write-CliDetail -Label 'Project' -Value $projectRoot -ValueTone 'Important'
Write-CliDetail -Label 'Tracked only' -Value $(if ($trackedOnly) { 'yes' } else { 'no' })
$copyPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$deletePaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)

foreach ($change in (Get-GitTrackedChanges -ProjectRoot $projectRoot -BaseCommit $sourceCommit)) {
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
    if ($path.Contains('\')) { throw "Unsafe Git path spelling: $path" }
    if (Test-TransferMetadata -Path $path) { throw "Reserved package metadata path: $path" }
    if (Test-TransferExcluded -Path $path) { [void]$copyPaths.Remove($path); continue }
    $sourcePath = Resolve-TransferPath -Root $projectRoot -Path $path
    if ($sourcePath.Equals($outputPath, $comparison)) { throw 'Output overlaps a changed source file.' }
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) { throw "Changed source is not a regular file: $path" }
}
foreach ($path in @($deletePaths)) {
    if ($path.Contains('\')) { throw "Unsafe Git path spelling: $path" }
    if (Test-TransferMetadata -Path $path) { throw "Reserved package metadata path: $path" }
    if (Test-TransferExcluded -Path $path) { [void]$deletePaths.Remove($path); continue }
    if ($path.StartsWith('#')) { throw "Deletion path cannot be represented in the manifest: $path" }
    $targetPath = Resolve-TransferPath -Root $projectRoot -Path $path
    if ($copyPaths.Contains($path) -and (Test-Path -LiteralPath $targetPath)) { [void]$deletePaths.Remove($path) }
}
Assert-TransferDisjoint -Paths @(@($copyPaths) + @($deletePaths))
if ($copyPaths.Count -eq 0 -and $deletePaths.Count -eq 0) {
    Write-CliSuccess -Message 'No changes to package'
    Write-CliDetail -Label 'Included' -Value '0 files'
    Write-CliDetail -Label 'Deleted' -Value '0 paths'
    return
}
if (Test-Path -LiteralPath $outputPath) {
    if (-not $force) { throw "Output already exists: $outputPath. Use --force to replace it." }
    if (-not (Test-Path -LiteralPath $outputPath -PathType Leaf)) { throw "Output is not a file: $outputPath" }
}
$outputParent = Split-Path $outputPath -Parent
[void][IO.Directory]::CreateDirectory($outputParent)
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$stagingRoot = Join-Path ([IO.Path]::GetTempPath()) ('apollo-package-' + [Guid]::NewGuid().ToString('N'))
$tempZip = Join-Path $outputParent ('.apollo-package-' + [Guid]::NewGuid().ToString('N') + '.tmp')
[void][IO.Directory]::CreateDirectory($stagingRoot)
$progressId = 21
try {
    $sortedCopyPaths = @($copyPaths | Sort-Object)
    if ($sortedCopyPaths.Count) { Write-CliStep -Message "Staging $($sortedCopyPaths.Count) file(s)" }
    for ($index = 0; $index -lt $sortedCopyPaths.Count; $index++) {
        $relativePath = $sortedCopyPaths[$index]
        $sourcePath = Resolve-TransferPath -Root $projectRoot -Path $relativePath
        $destinationPath = Join-Path $stagingRoot $relativePath
        [void][IO.Directory]::CreateDirectory((Split-Path $destinationPath -Parent))
        Copy-Item -LiteralPath $sourcePath -Destination $destinationPath -Force
        Write-CliProgress -Activity 'Packaging changes' -Status 'Staging files' -Current ($index + 1) -Total $sortedCopyPaths.Count -Id $progressId
    }
    if ($deletePaths.Count) {
        [IO.File]::WriteAllLines((Join-Path $stagingRoot $deleteManifestName), [string[]]@($deletePaths | Sort-Object), [Text.UTF8Encoding]::new($false))
    }
    $packageManifest = [ordered]@{
        schemaVersion = 1
        packageKind = 'apollo-change-package'
        projectName = 'Apollo'
        sourceCommit = $sourceCommit
        trackedOnly = $trackedOnly
        includedFiles = $copyPaths.Count
        deletedPaths = $deletePaths.Count
        createdAt = [DateTime]::UtcNow.ToString('o')
    }
    [IO.File]::WriteAllText((Join-Path $stagingRoot $packageManifestName), ($packageManifest | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Write-CliStep -Message 'Compressing archive'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($stagingRoot, $tempZip, [IO.Compression.CompressionLevel]::Optimal, $false)
    $archive = [IO.Compression.ZipFile]::OpenRead($tempZip)
    try {
        $expected = $copyPaths.Count + 1 + [int]($deletePaths.Count -gt 0)
        if ($archive.Entries.Count -ne $expected) { throw 'Unexpected package archive contents.' }
        foreach ($entry in $archive.Entries) { $stream = $entry.Open(); try { $stream.CopyTo([IO.Stream]::Null) } finally { $stream.Dispose() } }
    } finally { $archive.Dispose() }
    [IO.File]::Move($tempZip, $outputPath, $force)
} finally {
    Complete-CliProgress -Id $progressId -Activity 'Packaging changes'
    if (Test-Path -LiteralPath $tempZip) { Remove-Item -LiteralPath $tempZip -Force }
    if (Test-Path -LiteralPath $stagingRoot) { Remove-Item -LiteralPath $stagingRoot -Recurse -Force }
}
$stopwatch.Stop()
Write-CliSuccess -Message 'Package created'
Write-CliDetail -Label 'Archive' -Value $outputPath -ValueTone 'Important'
Write-CliDetail -Label 'Size' -Value (Format-CliByteSize -Bytes (Get-Item -LiteralPath $outputPath).Length)
Write-CliDetail -Label 'Included' -Value "$($copyPaths.Count) file(s)"
Write-CliDetail -Label 'Deleted' -Value "$($deletePaths.Count) path(s)"
Write-CliDetail -Label 'Elapsed' -Value ('{0:0.0}s' -f $stopwatch.Elapsed.TotalSeconds)
