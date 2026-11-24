#requires -Version 7.0
# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
# Modified for Apollo on 2026-11-24.

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "Cli.Common.ps1")
. (Join-Path $PSScriptRoot "Repository.Common.ps1")

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    mode = @{ Aliases = @('-Mode') }
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Join-Path $PSScriptRoot '..') }
    output = @{ Aliases = @('-o', '-OutputPath') }
}
if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\zip-project.ps1 --mode repo|workspace [options]

Options:
  --mode <repo|workspace>       Export committed Git source or the filtered workspace.
  -p, --project <path>          Project root (default: directory above scripts).
  -o, --output <path>           External ZIP path (default: apollo-<mode>(yyMMddHHmm).zip beside project).
  -h, --help                    Show this help.
'@
    return
}

if (-not $cli['mode']) { throw 'Specify --mode repo or --mode workspace.' }
$Mode = Resolve-CliChoice -Value $cli['mode'] -OptionName '--mode' -AllowedValues @('repo', 'workspace')
if (Test-RepositoryFileSystemRoot -Path $cli['project']) { throw 'Cannot export a filesystem root.' }
$ProjectRoot = Resolve-RepositoryRoot -Path $cli['project']
$ProjectName = Split-Path $ProjectRoot -Leaf

# Refuse linked ancestors: a lexically external output must not resolve into the project.
function Assert-UnlinkedPath {
    param([string]$Path)
    for ($current = $Path; $current; $current = [System.IO.Path]::GetDirectoryName($current)) {
        $item = Get-Item -LiteralPath $current -Force -ErrorAction SilentlyContinue
        if ($null -ne $item -and ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw "Symbolic links and reparse points are not supported: $current"
        }
    }
}
Assert-UnlinkedPath -Path $ProjectRoot

$SourceCommit = $null
$WorkingTreeDirty = $null
$gitRoot = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('rev-parse', '--show-toplevel') -AllowFailure
if ($gitRoot.ExitCode -eq 0 -and [System.IO.Path]::GetFullPath($gitRoot.StdOut.Trim()).Equals($ProjectRoot, (Get-RepositoryPathComparison))) {
    $head = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('rev-parse', '--verify', 'HEAD') -AllowFailure
    if ($head.ExitCode -eq 0) { $SourceCommit = $head.StdOut.Trim() }
    if ($SourceCommit -and $SourceCommit -notmatch '^(?:[0-9a-f]{40}|[0-9a-f]{64})$') { throw 'Invalid Git source commit.' }
    $status = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('status', '--porcelain', '-z', '--untracked-files=all')
    $WorkingTreeDirty = $status.StdOut.Length -gt 0
}
if ($Mode -eq 'repo' -and -not $SourceCommit) { throw 'Repo export requires a committed Git repository at the project root.' }

$OutputPath = if ($cli['output']) {
    [System.IO.Path]::GetFullPath([string]$cli['output'])
} else {
    $timestamp = (Get-Date).ToString('yyMMddHHmm')
    Join-Path (Split-Path $ProjectRoot -Parent) "apollo-$Mode($timestamp).zip"
}
if ([System.IO.Path]::GetExtension($OutputPath) -ine '.zip') { throw "Output path must end in .zip: $OutputPath" }
$projectPrefix = "$ProjectRoot$([System.IO.Path]::DirectorySeparatorChar)"
if ($OutputPath.Equals($ProjectRoot, (Get-RepositoryPathComparison)) -or $OutputPath.StartsWith($projectPrefix, (Get-RepositoryPathComparison))) {
    throw "Output ZIP must be outside the exported repository: $OutputPath"
}
Assert-UnlinkedPath -Path $OutputPath
if (Test-Path -LiteralPath $OutputPath) { throw "Output ZIP already exists: $OutputPath" }

$ManifestName = 'EXPORT_MANIFEST.json'
function Assert-SourcePaths {
    param([AllowEmptyCollection()][string[]]$Paths)
    if (@($Paths | Where-Object { $_ -match '^EXPORT_MANIFEST\.json(?:/|$)' }).Count -gt 0) {
        throw "Export already contains reserved manifest path $ManifestName."
    }
    if ($Mode -eq 'repo' -and @($Paths | Where-Object { $_ -match '^\.(?:plans|planning|project)(?:/|$)' }).Count -gt 0) {
        throw 'Repo export cannot contain committed local-only .plans, .planning or .project files.'
    }
}

# Match one path at a time, including directories before descending into them.
function ConvertTo-ZipIgnoreRegex {
    param([Parameter(Mandatory)][string]$Pattern)

    $normalized = $Pattern.Trim().Replace('\', '/')
    # The reference's bsdtar-compatible ^ anchor preserves root-only exclusions.
    $rootAnchored = $normalized.StartsWith('^')
    if ($rootAnchored) { $normalized = $normalized.Substring(1) }
    $anchored = $rootAnchored -or $normalized.StartsWith('/') -or $normalized.Trim('/').Contains('/')
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

function Test-ZipIgnored {
    param([string]$RelativePath, [bool]$IsDirectory, [object[]]$Rules)
    foreach ($rule in $Rules) {
        if (($IsDirectory -or -not $rule.DirectoryOnly) -and $rule.Regex.IsMatch($RelativePath)) { return $true }
    }
    return $false
}

function Get-WorkspaceFiles {
    $ignoreFile = Join-Path $ProjectRoot '.zipignore'
    if (-not (Test-Path -LiteralPath $ignoreFile -PathType Leaf)) { throw "Missing .zipignore: $ignoreFile" }
    $rules = @(
        foreach ($line in (Get-Content -LiteralPath $ignoreFile)) {
            $pattern = $line.Trim()
            if (-not $pattern -or $pattern.StartsWith('#')) { continue }
            if ($pattern.StartsWith('!')) { throw ".zipignore negation is not supported: $pattern" }
            ConvertTo-ZipIgnoreRegex -Pattern $pattern
        }
    )
    $directories = [System.Collections.Generic.Stack[string]]::new()
    $directories.Push($ProjectRoot)
    while ($directories.Count -gt 0) {
        foreach ($item in (Get-ChildItem -LiteralPath ($directories.Pop()) -Force)) {
            $relativePath = [System.IO.Path]::GetRelativePath($ProjectRoot, $item.FullName).Replace('\', '/')
            if (Test-ZipIgnored -RelativePath $relativePath -IsDirectory $item.PSIsContainer -Rules $rules) { continue }
            if ($relativePath -match '^EXPORT_MANIFEST\.json(?:/|$)') { throw "Export already contains reserved manifest path $ManifestName." }
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw "Cannot snapshot a symbolic link or reparse point: $relativePath" }
            if ($item.PSIsContainer) { $directories.Push($item.FullName) }
            else { [pscustomobject]@{ File = $item; RelativePath = $relativePath } }
        }
    }
}

$sourceArchive = $null
$inputArchive = $null
$temporaryArchive = $null
$outputParent = Split-Path $OutputPath -Parent
$progressActivity = "Packaging $ProjectName export"
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
Add-Type -AssemblyName System.IO.Compression.FileSystem
Write-CliHeading -Text "Exporting $ProjectName ($Mode)"
try {
    Write-CliStep -Message 'Creating source snapshot'
    if ($Mode -eq 'repo') {
        Assert-SourcePaths -Paths @(Get-GitNulValues -ProjectRoot $ProjectRoot -Arguments @('ls-tree', '-r', '--name-only', '-z', $SourceCommit))
        $sourceArchive = Join-Path ([System.IO.Path]::GetTempPath()) "apollo-source-$([Guid]::NewGuid().ToString('N')).zip"
        [void](Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('archive', '--format=zip', '-0', "--output=$sourceArchive", $SourceCommit))
        $inputArchive = [System.IO.Compression.ZipFile]::OpenRead($sourceArchive)
        $sourceEntries = @($inputArchive.Entries)
        Assert-SourcePaths -Paths @($sourceEntries | ForEach-Object { $_.FullName.Replace('\', '/') })
        $fileCount = @($sourceEntries | Where-Object { -not $_.FullName.EndsWith('/') }).Count
    } else {
        $sourceEntries = @(Get-WorkspaceFiles | Sort-Object RelativePath)
        $fileCount = $sourceEntries.Count
    }

    Write-CliStep -Message "Packaging $fileCount files"
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
    $temporaryArchive = Join-Path $outputParent ".apollo-export-$([Guid]::NewGuid().ToString('N')).tmp"
    $stream = [System.IO.File]::Open($temporaryArchive, [System.IO.FileMode]::CreateNew)
    try {
        $outputArchive = [System.IO.Compression.ZipArchive]::new($stream, [System.IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            $paths = [System.Collections.Generic.List[string]]::new()
            $processedFiles = 0
            $progressUpdateEvery = [Math]::Max(1, [int][Math]::Ceiling($fileCount / 100.0))
            foreach ($sourceEntry in $sourceEntries) {
                if ($Mode -eq 'repo') {
                    $name = $sourceEntry.FullName
                    $destinationEntry = $outputArchive.CreateEntry($name, [System.IO.Compression.CompressionLevel]::Optimal)
                    $destinationEntry.LastWriteTime = $sourceEntry.LastWriteTime
                    $destinationEntry.ExternalAttributes = $sourceEntry.ExternalAttributes
                    if ($name.EndsWith('/')) { continue }
                    $sourceStream = $sourceEntry.Open()
                } else {
                    $name = $sourceEntry.RelativePath
                    $destinationEntry = $outputArchive.CreateEntry($name, [System.IO.Compression.CompressionLevel]::Optimal)
                    $timestamp = [DateTimeOffset]$sourceEntry.File.LastWriteTimeUtc
                    $minimum = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
                    $maximum = [DateTimeOffset]::new(2107, 12, 31, 23, 59, 58, [TimeSpan]::Zero)
                    if ($timestamp -lt $minimum) { $timestamp = $minimum }
                    if ($timestamp -gt $maximum) { $timestamp = $maximum }
                    $destinationEntry.LastWriteTime = $timestamp
                    $destinationEntry.ExternalAttributes = [int]$sourceEntry.File.Attributes
                    $sourceStream = [System.IO.File]::OpenRead($sourceEntry.File.FullName)
                }
                try {
                    $destinationStream = $destinationEntry.Open()
                    try { $sourceStream.CopyTo($destinationStream) } finally { $destinationStream.Dispose() }
                } finally { $sourceStream.Dispose() }
                $paths.Add($name.Replace('\', '/'))
                $processedFiles++
                if ($processedFiles -eq 1 -or $processedFiles -eq $fileCount -or ($processedFiles % $progressUpdateEvery) -eq 0) {
                    Write-CliProgress -Activity $progressActivity -Status 'Files' -Current $processedFiles -Total $fileCount
                }
            }
            Complete-CliProgress -Activity $progressActivity
            Write-CliStep -Message 'Writing export manifest'
            $manifest = [ordered]@{
                schemaVersion = 1
                exportMode = $Mode
                sourceCommit = $SourceCommit
                workingTreeDirty = $WorkingTreeDirty
                trackedSourceBaseline = if ($Mode -eq 'repo') { 'git-commit' } elseif ($SourceCommit) { 'working-tree-over-commit' } else { 'working-tree' }
                includedLocalOnly = [ordered]@{
                    planning = @($paths | Where-Object { $_ -match '^\.(?:plans|planning)/.+' }).Count -gt 0
                    project = @($paths | Where-Object { $_ -match '^\.project/.+' }).Count -gt 0
                }
                createdAt = (Get-Date).ToUniversalTime().ToString('o')
            }
            $entry = $outputArchive.CreateEntry($ManifestName)
            $writer = [System.IO.StreamWriter]::new($entry.Open(), [System.Text.UTF8Encoding]::new($false))
            try { $writer.WriteLine(($manifest | ConvertTo-Json -Depth 4)) } finally { $writer.Dispose() }
        } finally { $outputArchive.Dispose() }
    } finally { $stream.Dispose() }

    Write-CliStep -Message 'Finalizing archive'
    $verify = [System.IO.Compression.ZipFile]::OpenRead($temporaryArchive)
    try {
        $actual = @($verify.Entries | Where-Object { -not $_.FullName.EndsWith('/') } | ForEach-Object { $_.FullName.Replace('\', '/') })
        if ($actual.Count -ne $fileCount + 1 -or @($actual | Where-Object { $_ -ceq $ManifestName }).Count -ne 1) {
            throw 'Export archive entry count or manifest is invalid.'
        }
        $expected = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        foreach ($path in $paths) { [void]$expected.Add($path) }
        [void]$expected.Add($ManifestName)
        if ($expected.Count -ne $actual.Count -or -not $expected.SetEquals([string[]]$actual)) { throw 'Export archive entries do not match the snapshot.' }
    } finally { $verify.Dispose() }
    # Same-directory, no-overwrite publication; a concurrent output is never replaced.
    [System.IO.File]::Move($temporaryArchive, $OutputPath)
    $stopwatch.Stop()
    $outputInfo = Get-Item -LiteralPath $OutputPath
    Write-Host ''
    Write-CliSuccess -Message 'Export complete'
    Write-CliDetail -Label 'Archive' -Value $OutputPath -ValueTone 'Important'
    Write-CliDetail -Label 'Size' -Value (Format-CliByteSize -Bytes $outputInfo.Length)
    Write-CliDetail -Label 'Files' -Value ([string]($fileCount + 1))
    Write-CliDetail -Label 'Mode' -Value $Mode
    Write-CliDetail -Label 'Commit' -Value $(if ($SourceCommit) { $SourceCommit } else { 'unavailable' })
    Write-CliDetail -Label 'Tree' -Value $(if ($null -eq $WorkingTreeDirty) { 'unavailable' } elseif ($WorkingTreeDirty) { 'dirty' } else { 'clean' })
    Write-CliDetail -Label 'Elapsed' -Value $stopwatch.Elapsed.ToString('g')
} finally {
    Complete-CliProgress -Activity $progressActivity
    if ($null -ne $inputArchive) { $inputArchive.Dispose() }
    foreach ($path in @($sourceArchive, $temporaryArchive)) {
        if ($path -and [System.IO.File]::Exists($path)) { [System.IO.File]::Delete($path) }
    }
}
