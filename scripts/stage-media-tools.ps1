#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Cli.Common.ps1')

function Assert-MediaText {
    param($Value, [string]$Name)
    if ($Value -isnot [string] -or [string]::IsNullOrWhiteSpace($Value)) {
        throw "Manifest requires nonempty string '$Name'."
    }
}

function Assert-MediaRelativePath {
    param($Value)
    Assert-MediaText $Value 'relative path'
    # Restrict to portable Windows names, excluding traversal, ADS and device names.
    foreach ($part in $Value.Split('/')) {
        if ($part -notmatch '\A[a-zA-Z0-9_][a-zA-Z0-9_.-]*\z' -or
            $part.EndsWith('.') -or $part -match '^(CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])(\.|$)') {
            throw "Unsafe archive or destination path: $Value"
        }
    }
}

function Read-MediaToolsManifest {
    param([string]$Path)
    $manifest = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -AsHashtable
    if ($manifest -isnot [System.Collections.IDictionary] -or
        $manifest.schemaVersion -isnot [long] -and $manifest.schemaVersion -isnot [int] -or
        $manifest.schemaVersion -ne 1 -or
        $manifest.platform -cne 'windows' -or $manifest.architecture -cne 'x86_64') {
        throw 'Manifest requires schemaVersion 1, platform windows and architecture x86_64.'
    }
    foreach ($field in @('sourceVersion', 'buildIdentity')) { Assert-MediaText $manifest[$field] $field }
    if ($manifest.sourceVersion -notmatch '^\d+\.\d+\.\d+$' -or
        -not $manifest.buildIdentity.StartsWith($manifest.sourceVersion + '-', [StringComparison]::Ordinal)) {
        throw 'Manifest requires an exact sourceVersion and a distribution-specific buildIdentity.'
    }
    if ($manifest.archive -isnot [System.Collections.IDictionary]) { throw 'Manifest requires archive object.' }
    $archive = $manifest.archive
    Assert-MediaRelativePath $archive.filename
    if ($archive.filename.Contains('/') -or -not $archive.filename.EndsWith('.zip')) { throw 'Archive filename must be a ZIP basename.' }
    if ($archive.sha256 -isnot [string] -or $archive.sha256 -cnotmatch '\A[0-9a-f]{64}\z') { throw 'Manifest SHA-256 must contain 64 lowercase hexadecimal characters.' }
    $uri = $null
    if (-not [Uri]::TryCreate([string]$archive.url, [UriKind]::Absolute, [ref]$uri) -or
        $uri.Scheme -ne 'https' -or $uri.Query -or $uri.Fragment -or $uri.UserInfo -or
        [Uri]::UnescapeDataString($uri.Segments[-1]) -cne $archive.filename -or
        -not $uri.AbsolutePath.Contains('/' + $manifest.sourceVersion + '/') -or
        -not $archive.filename.Contains($manifest.sourceVersion)) {
        throw 'Manifest requires a versioned HTTPS archive URL matching sourceVersion and filename.'
    }
    if ($manifest.executables -isnot [System.Collections.IDictionary]) { throw 'Manifest requires executable pair.' }
    foreach ($tool in @('ffmpeg', 'ffprobe')) {
        Assert-MediaRelativePath $manifest.executables[$tool]
        if (($manifest.executables[$tool].Split('/')[-1]) -cne "$tool.exe") { throw "Manifest requires $tool.exe." }
    }
    if ($manifest.requiredEncoders -isnot [array] -or $manifest.requiredEncoders.Count -eq 0) { throw 'Manifest requires encoder array.' }
    foreach ($encoder in $manifest.requiredEncoders) {
        if ($encoder -isnot [string] -or $encoder -cnotmatch '\A[a-z0-9_]+\z') { throw 'Invalid required encoder name.' }
    }
    foreach ($encoder in @('libx264', 'libx265', 'h264_nvenc', 'hevc_nvenc')) {
        if ($manifest.requiredEncoders -cnotcontains $encoder) { throw "Manifest must require encoder $encoder." }
    }
    if ($manifest.redistribution -isnot [array] -or $manifest.redistribution.Count -eq 0) { throw 'Manifest requires redistribution files.' }
    $destinations = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($item in $manifest.redistribution) {
        if ($item -isnot [System.Collections.IDictionary]) { throw 'Invalid redistribution entry.' }
        Assert-MediaRelativePath $item.source
        Assert-MediaRelativePath $item.destination
        Assert-MediaText $item.purpose 'redistribution purpose'
        if (-not $item.destination.StartsWith('licenses/ffmpeg/', [StringComparison]::Ordinal) -or
            -not $destinations.Add($item.destination)) { throw 'Redistribution destinations must be unique beneath licenses/ffmpeg/.' }
    }
    foreach ($required in @('licenses/ffmpeg/LICENSE', 'licenses/ffmpeg/README.txt')) {
        if (-not $destinations.Contains($required)) { throw "Manifest requires redistribution destination $required." }
    }
    if ($manifest.compatibility -isnot [System.Collections.IDictionary]) { throw 'Manifest requires compatibility baseline.' }
    foreach ($field in @('nvencApi', 'headersIdentity', 'testedWindowsDriver', 'testedGpu', 'evidence', 'notes')) {
        Assert-MediaText $manifest.compatibility[$field] "compatibility.$field"
    }
    if ($manifest.compatibility.nvencApi -notmatch '\A\d+\.\d+\z' -or
        $manifest.compatibility.testedWindowsDriver -notmatch '\A\d+\.\d+\z') {
        throw 'Compatibility requires numeric NVENC API and tested Windows driver versions.'
    }
    Assert-MediaRelativePath $manifest.compatibility.evidence
    return $manifest
}

function Invoke-MediaTool {
    param([string]$Path, [string[]]$Arguments)
    $info = [Diagnostics.ProcessStartInfo]::new($Path)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        [void]$process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "Media tool validation timed out: $Path"
        }
        $output = $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw "Media tool validation failed ($($process.ExitCode)): $Path`n$output" }
        return $output
    }
    finally { $process.Dispose() }
}

function Assert-StagedMediaTools {
    param([string]$Path, $Manifest)
    foreach ($tool in @('ffmpeg', 'ffprobe')) {
        $output = Invoke-MediaTool (Join-Path $Path "$tool.exe") @('-version')
        $expected = '(?m)^' + $tool + ' version ' + [regex]::Escape($Manifest.buildIdentity) + '(?:\s|$)'
        if ($output -cnotmatch $expected) { throw "Unexpected $tool version; expected $($Manifest.buildIdentity).`n$output" }
    }
    $inventory = Invoke-MediaTool (Join-Path $Path 'ffmpeg.exe') @('-hide_banner', '-encoders')
    foreach ($encoder in $Manifest.requiredEncoders) {
        if ($inventory -cnotmatch ('(?m)^\s*[VAS][A-Z.]{5}\s+' + [regex]::Escape($encoder) + '\s')) {
            throw "Staged FFmpeg lacks required encoder: $encoder"
        }
    }
}

function Publish-MediaTools {
    param([string]$ManifestPath, [string]$CachePath, [string]$OutputPath, [switch]$Offline)
    $manifest = Read-MediaToolsManifest $ManifestPath
    $output = [IO.Path]::GetFullPath($OutputPath)
    # Publish only to a new destination. Never delete caller-owned release artifacts.
    if (Test-Path -LiteralPath $output) { throw "Media-tool output already exists: $output" }
    $cache = [IO.Path]::GetFullPath($CachePath)
    $archivePath = Join-Path $cache $manifest.archive.filename
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        if ($Offline) { throw "Offline cache miss: $archivePath" }
        New-Item -ItemType Directory -Path $cache -Force | Out-Null
        $download = Join-Path $cache ('.download-' + [Guid]::NewGuid().ToString('N'))
        try {
            Invoke-WebRequest -Uri $manifest.archive.url -OutFile $download -TimeoutSec 300
            if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant() -cne $manifest.archive.sha256) {
                throw 'Downloaded archive SHA-256 mismatch; extraction was not attempted.'
            }
            [IO.File]::Move($download, $archivePath)
        }
        finally { if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download -Force } }
    }
    # Hold the verified file open without write sharing through extraction.
    $stream = [IO.File]::Open($archivePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $scratch = $null
    try {
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $hash = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
        finally { $sha.Dispose() }
        if ($hash -cne $manifest.archive.sha256) { throw 'Cached archive SHA-256 mismatch; extraction was not attempted.' }
        $stream.Position = 0
        $parent = [IO.Path]::GetDirectoryName($output)
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
        $scratch = Join-Path $parent ('.media-tools-' + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $scratch | Out-Null
        $payload = Join-Path $scratch 'payload'
        New-Item -ItemType Directory -Path $payload | Out-Null
        $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read, $true)
        try {
            $entries = [Collections.Generic.Dictionary[string,object]]::new([StringComparer]::OrdinalIgnoreCase)
            foreach ($entry in $zip.Entries) {
                if ($entry.FullName.EndsWith('/')) { continue }
                Assert-MediaRelativePath $entry.FullName
                if (-not $entries.TryAdd($entry.FullName, $entry)) { throw "Duplicate archive entry: $($entry.FullName)" }
            }
            $files = @(
                @{ source = $manifest.executables.ffmpeg; destination = 'ffmpeg.exe' }
                @{ source = $manifest.executables.ffprobe; destination = 'ffprobe.exe' }
            ) + @($manifest.redistribution)
            foreach ($file in $files) {
                if (-not $entries.ContainsKey($file.source)) { throw "Required archive file missing: $($file.source)" }
                $entry = $entries[$file.source]
                if ($entry.Length -eq 0 -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) {
                    throw "Required archive file is empty or a symlink: $($file.source)"
                }
                $destination = Join-Path $payload $file.destination
                New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($destination)) -Force | Out-Null
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destination)
            }
        }
        finally { $zip.Dispose() }
        Assert-StagedMediaTools $payload $manifest
        [IO.Directory]::Move($payload, $output)
    }
    finally {
        $stream.Dispose()
        # Only the unique directory created by this invocation is removed.
        if ($null -ne $scratch -and (Test-Path -LiteralPath $scratch)) { Remove-Item -LiteralPath $scratch -Recurse -Force }
    }
    return $output
}

function Invoke-StageMediaToolsCli {
    param([AllowEmptyCollection()][object[]]$Arguments)
    $root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        manifest = @{ Default = (Join-Path $root 'distribution/media-tools.json') }
        cache = @{ Default = (Join-Path $root '.scratch/media-tools-cache') }
        output = @{ Default = (Join-Path $root '.scratch/staged-media-tools') }
        offline = @{ Kind = 'Switch' }
    }
    if ($cli['help']) {
        Write-CliHelp @'
Usage: .\scripts\stage-media-tools.ps1 [options]
  --manifest <path>  Pinned manifest. Defaults to distribution/media-tools.json.
  --cache <path>     Archive cache. Defaults to .scratch/media-tools-cache.
  --output <path>    New output directory. Defaults to .scratch/staged-media-tools.
  --offline          Require a checksum-valid cached archive; never download.
  --help             Show help.
Explicit relative paths resolve from the current directory. Existing output is refused.
Stages ffmpeg.exe, ffprobe.exe and licenses/ffmpeg/* only. No GPU is required.
'@
        return
    }
    $output = Publish-MediaTools -ManifestPath $cli['manifest'] -CachePath $cli['cache'] -OutputPath $cli['output'] -Offline:$cli['offline']
    Write-Host "Staged media tools: $output"
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-StageMediaToolsCli -Arguments $args }
