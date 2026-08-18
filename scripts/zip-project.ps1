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
'@
    return
}

$projectRoot = Resolve-RepositoryRoot -Path (Join-Path $PSScriptRoot '..')
$projectSlug = Get-RepositorySlug -ProjectRoot $projectRoot
$timestamp = Get-Date -Format 'yyMMddHHmm'
$outputFileName = "$projectSlug($timestamp).zip"
$outputPath = Join-Path (Split-Path $projectRoot -Parent) $outputFileName
$ignoreFile = Join-Path $projectRoot '.zipignore'

function ConvertTo-ZipIgnoreRegex {
    param([Parameter(Mandatory)][string]$Pattern)

    $normalized = $Pattern.Trim().Replace('\', '/').TrimStart('/')
    $directoryPattern = $normalized.EndsWith('/')
    $normalized = $normalized.TrimEnd('/')

    if ([string]::IsNullOrWhiteSpace($normalized)) {
        return $null
    }

    $marker = '__DOUBLE_STAR__'
    $escaped = [regex]::Escape($normalized)
    $escaped = $escaped.Replace('\*\*', $marker)
    $escaped = $escaped.Replace('\*', '[^/]*')
    $escaped = $escaped.Replace('\?', '[^/]')
    $escaped = $escaped.Replace($marker, '.*')

    if ($normalized.Contains('/')) {
        return '^' + $escaped + '(?:/.*)?$'
    }

    if ($directoryPattern) {
        return '(^|/)' + $escaped + '(?:/.*)$'
    }

    return '(^|/)' + $escaped + '(?:$|/.*)'
}

$ignoreRegexes = [System.Collections.Generic.List[regex]]::new()
if (Test-Path -LiteralPath $ignoreFile -PathType Leaf) {
    $regexOptions = if ($IsWindows) {
        [System.Text.RegularExpressions.RegexOptions]::IgnoreCase
    }
    else {
        [System.Text.RegularExpressions.RegexOptions]::None
    }

    foreach ($line in (Get-Content -LiteralPath $ignoreFile)) {
        $pattern = $line.Trim()
        if (-not $pattern -or $pattern.StartsWith('#')) {
            continue
        }
        if ($pattern.StartsWith('!')) {
            throw ".zipignore negation is not supported: $pattern"
        }

        $regexText = ConvertTo-ZipIgnoreRegex -Pattern $pattern
        if ($regexText) {
            $ignoreRegexes.Add([regex]::new($regexText, $regexOptions))
        }
    }
}

function Test-ZipIgnored {
    param([Parameter(Mandatory)][string]$RelativePath)

    $normalized = $RelativePath.Replace('\', '/')
    foreach ($regex in $ignoreRegexes) {
        if ($regex.IsMatch($normalized)) {
            return $true
        }
    }
    return $false
}

if (Test-Path -LiteralPath $outputPath) {
    Remove-Item -LiteralPath $outputPath -Force
}

Add-Type -AssemblyName System.IO.Compression
try {
    $stream = [System.IO.File]::Open($outputPath, [System.IO.FileMode]::CreateNew)
    try {
        $archive = [System.IO.Compression.ZipArchive]::new($stream, [System.IO.Compression.ZipArchiveMode]::Create, $false)
        try {
            $files = @(Get-ChildItem -LiteralPath $projectRoot -File -Recurse -Force |
                ForEach-Object {
                    $relativePath = [System.IO.Path]::GetRelativePath($projectRoot, $_.FullName).Replace('\', '/')
                    [pscustomobject]@{ File = $_; RelativePath = $relativePath }
                } |
                Where-Object { $_.RelativePath -ne $outputFileName -and -not (Test-ZipIgnored -RelativePath $_.RelativePath) } |
                Sort-Object RelativePath)

            foreach ($item in $files) {
                $entry = $archive.CreateEntry($item.RelativePath, [System.IO.Compression.CompressionLevel]::Optimal)
                $entryTimestamp = [DateTimeOffset]$item.File.LastWriteTimeUtc
                $minimumZipTimestamp = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
                if ($entryTimestamp -lt $minimumZipTimestamp) { $entryTimestamp = $minimumZipTimestamp }
                $entry.LastWriteTime = $entryTimestamp
                $entryStream = $entry.Open()
                try {
                    $sourceStream = [System.IO.File]::OpenRead($item.File.FullName)
                    try { $sourceStream.CopyTo($entryStream) } finally { $sourceStream.Dispose() }
                }
                finally { $entryStream.Dispose() }
            }
        }
        finally { $archive.Dispose() }
    }
    finally { $stream.Dispose() }
}
catch {
    if (Test-Path -LiteralPath $outputPath) {
        Remove-Item -LiteralPath $outputPath -Force
    }
    throw
}

Write-Host "Created: $outputPath"
