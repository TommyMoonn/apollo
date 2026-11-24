#requires -Version 7.0
# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'WindowsRelease.Common.ps1')
. (Join-Path $PSScriptRoot 'stage-media-tools.ps1')
. (Join-Path $PSScriptRoot 'check-release.ps1')

function Assert-ReleaseKeys($Object, [string[]]$Names) {
    if ($Object -isnot [System.Collections.IDictionary] -or $Object.Count -ne $Names.Count) { throw 'Invalid provenance object shape.' }
    foreach ($key in $Object.Keys) { if ($Names -cnotcontains $key) { throw "Unexpected provenance key: $key" } }
}

function Assert-UniqueReleaseJson($Element) {
    switch ($Element.ValueKind.ToString()) {
        'Object' {
            $keys = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
            foreach ($property in $Element.EnumerateObject()) {
                if (-not $keys.Add($property.Name)) { throw "Duplicate provenance key: $($property.Name)" }
                Assert-UniqueReleaseJson $property.Value
            }
        }
        'Array' { foreach ($value in $Element.EnumerateArray()) { Assert-UniqueReleaseJson $value } }
    }
}

function Read-ReleaseProvenance([string]$Path) {
    $source = [IO.File]::ReadAllText($Path)
    $document = [System.Text.Json.JsonDocument]::Parse($source, [System.Text.Json.JsonDocumentOptions]::new())
    try { Assert-UniqueReleaseJson $document.RootElement }
    finally { $document.Dispose() }
    return ($source | ConvertFrom-Json -AsHashtable)
}

function Assert-ReleaseRecord($Record, $Actual, [string]$Name) {
    Assert-ReleaseKeys $Record @('sha256', 'size')
    if ($Record.sha256 -isnot [string] -or $Record.sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        ($Record.size -isnot [long] -and $Record.size -isnot [int]) -or $Record.size -le 0 -or
        $Record.size -ne $Actual.size -or $Record.sha256 -cne $Actual.sha256) { throw "Release digest or size mismatch: $Name" }
}

function Assert-WindowsReleaseArtifacts {
    param([string]$ProjectRoot, [string]$ArtifactsDirectory, [string]$ExpectedVersion, [string]$ExpectedSha, [switch]$Local, [switch]$PassThru)
    Assert-StableReleaseVersion $ExpectedVersion
    $release = Invoke-ReleaseValidation -ProjectRoot $ProjectRoot
    if ($release.Version -cne $ExpectedVersion) { throw 'Expected release version does not match the checkout.' }
    if ($Local) {
        if ($ExpectedSha) { throw '--local cannot be combined with --expected-sha.' }
    } else { Assert-ReleaseCandidateCheckout -ProjectRoot $ProjectRoot -ExpectedSha $ExpectedSha }
    Assert-ReleaseDirectoryContents -Directory $ArtifactsDirectory -Exact
    $provenance = Read-ReleaseProvenance (Join-Path $ArtifactsDirectory 'release-provenance.json')
    Assert-ReleaseKeys $provenance @('schemaVersion', 'candidateSha', 'version', 'target', 'rustVersion', 'nsisVersion', 'mediaArchive', 'assets', 'payload')
    if (($provenance.schemaVersion -isnot [long] -and $provenance.schemaVersion -isnot [int]) -or $provenance.schemaVersion -ne 1) { throw 'Unsupported provenance schema.' }
    foreach ($name in @('version','target','rustVersion','nsisVersion')) { if ($provenance[$name] -isnot [string]) { throw "Invalid provenance field: $name" } }
    if ($provenance.version -cne $ExpectedVersion -or $provenance.target -cne 'x86_64-pc-windows-msvc' -or
        $provenance.rustVersion -cne (Get-ReleaseRustPin $ProjectRoot) -or $provenance.nsisVersion -cne '3.11') { throw 'Release build identity mismatch.' }
    if ($Local) {
        if ($null -ne $provenance.candidateSha) { throw 'Candidate artifacts require explicit expected SHA verification.' }
    } elseif ($provenance.candidateSha -isnot [string] -or $provenance.candidateSha -cne $ExpectedSha) { throw 'Candidate SHA mismatch.' }

    $manifestPath = Join-Path $ProjectRoot 'distribution/media-tools.json'
    $media = Read-MediaToolsManifest $manifestPath
    Assert-ReleaseKeys $provenance.mediaArchive @('filename','url','sha256','buildIdentity')
    foreach ($name in @('filename','url','sha256')) {
        if ($provenance.mediaArchive[$name] -isnot [string] -or $provenance.mediaArchive[$name] -cne $media.archive[$name]) { throw "Pinned media archive mismatch: $name" }
    }
    if ($provenance.mediaArchive.buildIdentity -isnot [string] -or $provenance.mediaArchive.buildIdentity -cne $media.buildIdentity) { throw 'Pinned media build identity mismatch.' }

    $assetNames = @(Get-WindowsPublicAssetNames)
    $binaryNames = @($assetNames | Where-Object { $_ -cne 'SHA256SUMS.txt' })
    Assert-ReleaseKeys $provenance.assets $assetNames
    $checksums = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
    foreach ($line in [IO.File]::ReadAllLines((Join-Path $ArtifactsDirectory 'SHA256SUMS.txt'))) {
        if ($line -cnotmatch '\A([0-9a-f]{64})  (.+)\z' -or $binaryNames -cnotcontains $Matches[2] -or -not $checksums.TryAdd($Matches[2], $Matches[1])) { throw 'Invalid or duplicate checksum entry.' }
    }
    if ($checksums.Count -ne $binaryNames.Count) { throw 'Incomplete checksum coverage.' }
    foreach ($name in $assetNames) {
        $actual = Get-ReleaseFileRecord (Join-Path $ArtifactsDirectory $name)
        Assert-ReleaseRecord $provenance.assets[$name] $actual $name
        if ($binaryNames -ccontains $name -and $checksums[$name] -cne $actual.sha256) { throw "Checksum mismatch: $name" }
    }

    $payloadNames = @(Get-WindowsPayloadPaths $media)
    Assert-ReleaseKeys $provenance.payload $payloadNames
    foreach ($name in @('LICENSE','THIRD_PARTY_NOTICES.md','licenses/ffmpeg/media-tools.json')) {
        $trusted = if ($name -ceq 'licenses/ffmpeg/media-tools.json') { $manifestPath } else { Join-Path $ProjectRoot $name }
        Assert-ReleaseRecord $provenance.payload[$name] (Get-ReleaseFileRecord $trusted) $name
    }

    # Never extract archive-supplied paths. Only the known Apollo executable is copied
    # to a disposable fixed filename for Windows resource inspection; it is not run.
    $scratch = Join-Path ([IO.Path]::GetTempPath()) ('apollo-verify-' + [guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($scratch)
    try {
        $archive = [IO.Compression.ZipFile]::OpenRead((Join-Path $ArtifactsDirectory 'Apollo-windows-x64.zip'))
        try {
            $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
            $buffer = [byte[]]::new(65536)
            foreach ($entry in $archive.Entries) {
                $name = $entry.FullName
                if ($payloadNames -cnotcontains $name -or -not $seen.Add($name) -or
                    ($entry.ExternalAttributes -band 0x410) -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -notin @(0, 0x8000)) { throw "Unexpected, linked or duplicate ZIP entry: $name" }
                $record = $provenance.payload[$name]
                Assert-ReleaseKeys $record @('sha256','size')
                if (($record.size -isnot [long] -and $record.size -isnot [int]) -or $record.size -le 0 -or $record.size -ne $entry.Length) { throw "ZIP payload size mismatch: $name" }
                $stream = $entry.Open()
                $hasher = [Security.Cryptography.IncrementalHash]::CreateHash([Security.Cryptography.HashAlgorithmName]::SHA256)
                $exe = $null
                $destination = $null
                $size = 0L
                try {
                    if ($name -ceq 'Apollo.exe') {
                        $exe = Join-Path $scratch 'Apollo.exe'
                        $destination = [IO.File]::Open($exe, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
                    }
                    while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                        $size += $read
                        if ($size -gt $record.size) { throw "ZIP payload exceeds recorded size: $name" }
                        $hasher.AppendData($buffer, 0, $read)
                        if ($null -ne $destination) { $destination.Write($buffer, 0, $read) }
                    }
                    $hash = [BitConverter]::ToString($hasher.GetHashAndReset()).Replace('-', '').ToLowerInvariant()
                } finally {
                    if ($null -ne $destination) { $destination.Dispose() }
                    $hasher.Dispose()
                    $stream.Dispose()
                }
                Assert-ReleaseRecord $record @{ sha256 = $hash; size = $size } $name
                if ($null -ne $exe) { Assert-ApolloResourceVersion -Path $exe -Version $ExpectedVersion }
            }
            if ($seen.Count -ne $payloadNames.Count) { throw 'Portable ZIP payload is incomplete.' }
        } finally { $archive.Dispose() }
    } finally { Remove-Item -LiteralPath $scratch -Recurse -Force }
    Write-CliSuccess 'Windows release artifacts verified'
    if ($Local) { Write-CliWarning 'Local artifact set; no candidate identity or publication eligibility asserted.' }
    if ($PassThru) { return $provenance }
}

function Invoke-VerifyWindowsReleaseCli {
    param([AllowEmptyCollection()][object[]]$Arguments)
    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        project = @{ Aliases = @('-p','-ProjectRoot'); Default = (Join-Path $PSScriptRoot '..') }
        'artifacts-dir' = @{ Default = 'artifacts/windows' }
        'expected-sha' = @{}
        'expected-version' = @{}
        local = @{ Kind = 'Switch' }
    }
    if ($cli['help']) {
        Write-CliHelp @'
Usage: verify-windows-release.ps1 --expected-version X.Y.Z (--expected-sha SHA | --local) [options]
  --artifacts-dir <path>  Staged or downloaded directory (default artifacts/windows).
  --project <path>       Trusted candidate checkout; relative paths resolve from here.
  --expected-sha <sha>   Full candidate commit ID.
  --expected-version <v> Expected stable Cargo version.
  --local                Verify non-candidate artifacts only.
  --help                 Show help.
'@
        return
    }
    $root = Resolve-RepositoryRoot -Path $cli.project
    $directory = Resolve-ReleasePath -ProjectRoot $root -Path $cli['artifacts-dir'] -OptionName '--artifacts-dir'
    Assert-WindowsReleaseArtifacts -ProjectRoot $root -ArtifactsDirectory $directory -ExpectedVersion $cli['expected-version'] -ExpectedSha $cli['expected-sha'] -Local:$cli.local
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-VerifyWindowsReleaseCli -Arguments $args }
