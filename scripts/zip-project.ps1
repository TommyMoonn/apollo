#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{}
if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\zip-project.ps1

Options:
  -h, --help                    Show this help.

Creates a filtered workspace ZIP beside the project using .zipignore.
'@
    return
}

$projectRoot = Resolve-RepositoryRoot -Path (Join-Path $PSScriptRoot '..')
$projectSlug = Get-RepositorySlug -ProjectRoot $projectRoot
$timestamp = Get-Date -Format 'yyMMddHHmm'
$outputFileName = "$projectSlug($timestamp).zip"
$outputPath = Join-Path (Split-Path $projectRoot -Parent) $outputFileName
$ignoreFile = Join-Path $projectRoot '.zipignore'

# Match one path at a time, including directories before descending into them.
function ConvertTo-ZipIgnoreRegex {
    param([Parameter(Mandatory)][string]$Pattern)

    $normalized = $Pattern.Trim().Replace('\', '/')
    $anchored = $normalized.StartsWith('/') -or $normalized.Trim('/').Contains('/')
    $directoryOnly = $normalized.EndsWith('/')
    $normalized = $normalized.Trim('/')
    if (-not $normalized) { throw "Empty .zipignore pattern: $Pattern" }

    $escaped = [System.Text.StringBuilder]::new()
    for ($i = 0; $i -lt $normalized.Length; $i++) {
        $character = $normalized[$i]
        if ($character -eq '*') {
            if ($i + 1 -lt $normalized.Length -and $normalized[$i + 1] -eq '*') {
                $i++
                if ($i + 1 -lt $normalized.Length -and $normalized[$i + 1] -eq '/') {
                    [void]$escaped.Append('(?:.*/)?')
                    $i++
                }
                else { [void]$escaped.Append('.*') }
            }
            else { [void]$escaped.Append('[^/]*') }
        }
        elseif ($character -eq '?') { [void]$escaped.Append('[^/]') }
        else { [void]$escaped.Append([regex]::Escape([string]$character)) }
    }
    $prefix = if ($anchored) { '^' } else { '(^|/)' }
    $options = [System.Text.RegularExpressions.RegexOptions]::CultureInvariant
    if ($IsWindows) { $options = $options -bor [System.Text.RegularExpressions.RegexOptions]::IgnoreCase }
    return [pscustomobject]@{
        Regex = [regex]::new($prefix + $escaped.ToString() + '$', $options)
        DirectoryOnly = $directoryOnly
    }
}

if (-not (Test-Path -LiteralPath $ignoreFile -PathType Leaf)) {
    throw "Missing .zipignore: $ignoreFile"
}
$ignoreRules = @(
    foreach ($line in (Get-Content -LiteralPath $ignoreFile)) {
        $pattern = $line.Trim()
        if (-not $pattern -or $pattern.StartsWith('#')) { continue }
        if ($pattern.StartsWith('!')) { throw ".zipignore negation is not supported: $pattern" }
        ConvertTo-ZipIgnoreRegex -Pattern $pattern
    }
)

function Test-ZipIgnored {
    param([string]$RelativePath, [bool]$IsDirectory)
    foreach ($rule in $ignoreRules) {
        if (($IsDirectory -or -not $rule.DirectoryOnly) -and $rule.Regex.IsMatch($RelativePath)) { return $true }
    }
    return $false
}

$files = [System.Collections.Generic.List[object]]::new()
$directories = [System.Collections.Generic.Stack[string]]::new()
$directories.Push($projectRoot)
$progressTimer = [System.Diagnostics.Stopwatch]::StartNew()
$temporaryPath = "$outputPath.$([Guid]::NewGuid().ToString('N')).tmp"
$activity = 'Workspace snapshot'

try {
    Write-Host 'Scanning workspace...'
    while ($directories.Count -gt 0) {
        $directory = $directories.Pop()
        foreach ($item in (Get-ChildItem -LiteralPath $directory -Force)) {
            $relativePath = [System.IO.Path]::GetRelativePath($projectRoot, $item.FullName).Replace('\', '/')
            if (Test-ZipIgnored -RelativePath $relativePath -IsDirectory $item.PSIsContainer) { continue }
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw "Cannot snapshot a symbolic link or reparse point: $relativePath"
            }
            if ($item.PSIsContainer) { $directories.Push($item.FullName) }
            else { $files.Add([pscustomobject]@{ File = $item; RelativePath = $relativePath }) }
        }
        if ($progressTimer.ElapsedMilliseconds -ge 150) {
            Write-CliProgress -Activity $activity -Status "Scanning: $($files.Count) files found"
            $progressTimer.Restart()
        }
    }

    Write-Host "Compressing $($files.Count) files..."
    Add-Type -AssemblyName System.IO.Compression
    $stream = [System.IO.File]::Open($temporaryPath, [System.IO.FileMode]::CreateNew)
    try {
        $archive = [System.IO.Compression.ZipArchive]::new($stream, [System.IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            $current = 0
            foreach ($item in ($files | Sort-Object RelativePath)) {
                $entry = $archive.CreateEntry($item.RelativePath, [System.IO.Compression.CompressionLevel]::Optimal)
                $timestamp = [DateTimeOffset]$item.File.LastWriteTimeUtc
                $minimum = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
                $maximum = [DateTimeOffset]::new(2107, 12, 31, 23, 59, 58, [TimeSpan]::Zero)
                if ($timestamp -lt $minimum) { $timestamp = $minimum }
                if ($timestamp -gt $maximum) { $timestamp = $maximum }
                $entry.LastWriteTime = $timestamp
                $entryStream = $entry.Open()
                try {
                    $sourceStream = [System.IO.File]::OpenRead($item.File.FullName)
                    try { $sourceStream.CopyTo($entryStream) } finally { $sourceStream.Dispose() }
                }
                finally { $entryStream.Dispose() }
                $current++
                if ($progressTimer.ElapsedMilliseconds -ge 150 -or $current -eq $files.Count) {
                    Write-CliProgress -Activity $activity -Status "Compressing: $current / $($files.Count) files" -Current $current -Total $files.Count
                    $progressTimer.Restart()
                }
            }
        }
        finally { $archive.Dispose() }
    }
    finally { $stream.Dispose() }

    # Only replace an existing same-minute snapshot after the new ZIP is complete.
    [System.IO.File]::Move($temporaryPath, $outputPath, $true)
}
finally {
    if ([System.IO.File]::Exists($temporaryPath)) { [System.IO.File]::Delete($temporaryPath) }
    Complete-CliProgress -Activity $activity
}

Write-Host "Created: $outputPath"
