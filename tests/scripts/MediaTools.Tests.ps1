#requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Test.Common.ps1')
. (Join-Path $PSScriptRoot '../../scripts/stage-media-tools.ps1')

$pinnedManifest = Join-Path $PSScriptRoot '../../distribution/media-tools.json'
$nativeRunner = ${function:Invoke-MediaTool}

function New-MediaFixture {
    $root = New-TestDirectory -Name 'media tools'
    $manifest = Get-Content $pinnedManifest -Raw | ConvertFrom-Json -AsHashtable
    return @{ Root = $root; Manifest = $manifest; Missing = ''; Extra = ''; VersionFailure = ''; MissingEncoder = ''; NativeFailure = $false }
}

function Write-MediaFixture {
    param($Fixture)
    $archive = Join-Path $Fixture.Root $Fixture.Manifest.archive.filename
    $zip = [IO.Compression.ZipFile]::Open($archive, [IO.Compression.ZipArchiveMode]::Create)
    try {
        $names = @($Fixture.Manifest.executables.ffmpeg, $Fixture.Manifest.executables.ffprobe) +
            @($Fixture.Manifest.redistribution | ForEach-Object { $_.source }) + @('unrelated/ffplay.exe')
        if ($Fixture.Extra) { $names += $Fixture.Extra }
        foreach ($name in $names) {
            if ($name -eq $Fixture.Missing) { continue }
            $writer = [IO.StreamWriter]::new($zip.CreateEntry($name).Open())
            try { $writer.Write("fixture bytes: $name") } finally { $writer.Dispose() }
        }
    }
    finally { $zip.Dispose() }
    $Fixture.Manifest.archive.sha256 = (Get-FileHash $archive).Hash.ToLowerInvariant()
    $Fixture.Manifest | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $Fixture.Root 'manifest.json')
}

# Exercise staging with a controlled process boundary. Fixtures are ZIPs, never host media tools.
function Invoke-MediaTool {
    param([string]$Path, [string[]]$Arguments)
    Assert-True (Test-Path -LiteralPath $Path -PathType Leaf) 'Validation must use the staged executable.'
    if ($fixture.NativeFailure) { throw 'Media tool validation failed (17): fixture failure' }
    $tool = [IO.Path]::GetFileNameWithoutExtension($Path)
    if ($Arguments -contains '-version') {
        if ($fixture.VersionFailure -eq $tool) { return "$tool version 9.0.0-other-build" }
        return "$tool version $($fixture.Manifest.buildIdentity) Copyright"
    }
    return (($fixture.Manifest.requiredEncoders | Where-Object { $_ -cne $fixture.MissingEncoder } |
        ForEach-Object { " V....D $_ fixture encoder" }) -join "`n") + "`n"
}

function Invoke-FixtureStage {
    Publish-MediaTools -ManifestPath (Join-Path $fixture.Root 'manifest.json') -CachePath $fixture.Root -OutputPath (Join-Path $fixture.Root 'output') -Offline
}

Invoke-TestCase 'repository manifest satisfies pinned distribution contract' {
    $manifest = Read-MediaToolsManifest $pinnedManifest
    Assert-Equal '7.0.2' $manifest.sourceVersion 'Initial source version must remain 7.0.2.'
}

foreach ($field in @('sourceVersion', 'buildIdentity', 'archive', 'executables', 'redistribution', 'requiredEncoders', 'compatibility')) {
    Invoke-TestCase "manifest rejects missing $field" {
        $fixture = New-MediaFixture
        try {
            $fixture.Manifest.Remove($field)
            $path = Join-Path $fixture.Root 'manifest.json'
            $fixture.Manifest | ConvertTo-Json -Depth 10 | Set-Content $path
            Assert-Throws { Read-MediaToolsManifest $path } '*'
        }
        finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
    }
}

$invalidCases = @(
    @{ Name = 'schema'; Edit = { $fixture.Manifest.schemaVersion = 2 } }
    @{ Name = 'schema type'; Edit = { $fixture.Manifest.schemaVersion = '1' } }
    @{ Name = 'architecture'; Edit = { $fixture.Manifest.architecture = 'arm64' } }
    @{ Name = 'source-only identity'; Edit = { $fixture.Manifest.buildIdentity = '7.0.2' } }
    @{ Name = 'checksum format'; Edit = { $fixture.Manifest.archive.sha256 = 'not-a-checksum' } }
    @{ Name = 'moving URL'; Edit = { $fixture.Manifest.archive.url = 'https://example.com/latest/ffmpeg.zip' } }
    @{ Name = 'path traversal'; Edit = { $fixture.Manifest.executables.ffmpeg = '../ffmpeg.exe' } }
    @{ Name = 'license collision'; Edit = { $fixture.Manifest.redistribution[1].destination = $fixture.Manifest.redistribution[0].destination } }
    @{ Name = 'omitted license'; Edit = { $fixture.Manifest.redistribution = @($fixture.Manifest.redistribution[1]) } }
)
foreach ($case in $invalidCases) {
    Invoke-TestCase "manifest rejects $($case.Name)" {
        $fixture = New-MediaFixture
        try {
            & $case.Edit
            $path = Join-Path $fixture.Root 'manifest.json'
            $fixture.Manifest | ConvertTo-Json -Depth 10 | Set-Content $path
            Assert-Throws { Read-MediaToolsManifest $path } '*'
        }
        finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
    }
}

Invoke-TestCase 'offline cache hit stages only pair and byte-identical redistribution files' {
    $fixture = New-MediaFixture
    try {
        Write-MediaFixture $fixture
        $output = Invoke-FixtureStage
        $files = @(Get-ChildItem $output -Recurse -File | ForEach-Object { [IO.Path]::GetRelativePath($output, $_.FullName).Replace('\', '/') } | Sort-Object)
        Assert-Equal @('ffmpeg.exe', 'ffprobe.exe', 'licenses/ffmpeg/LICENSE', 'licenses/ffmpeg/media-tools.json', 'licenses/ffmpeg/README.txt') $files 'Only allowlisted files and pinned metadata may be staged.'
        Assert-Equal (Get-Content (Join-Path $fixture.Root 'manifest.json') -Raw) (Get-Content (Join-Path $output 'licenses/ffmpeg/media-tools.json') -Raw) 'Staged metadata must match the pinned manifest exactly.'
        foreach ($item in $fixture.Manifest.redistribution) {
            Assert-Equal "fixture bytes: $($item.source)" ([IO.File]::ReadAllText((Join-Path $output $item.destination))) 'Redistribution bytes must survive extraction.'
        }
        Assert-Throws { Invoke-FixtureStage } '*output already exists*'
    }
    finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
}

Invoke-TestCase 'offline cache miss never fetches or publishes' {
    $fixture = New-MediaFixture
    try {
        $fixture.Manifest | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $fixture.Root 'manifest.json')
        Assert-Throws { Invoke-FixtureStage } '*Offline cache miss*'
        Assert-True (-not (Test-Path (Join-Path $fixture.Root 'output'))) 'Failure must not publish.'
    }
    finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
}

Invoke-TestCase 'bad checksum fails before opening even a malformed ZIP' {
    $fixture = New-MediaFixture
    try {
        $fixture.Manifest | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $fixture.Root 'manifest.json')
        'not a ZIP' | Set-Content (Join-Path $fixture.Root $fixture.Manifest.archive.filename)
        Assert-Throws { Invoke-FixtureStage } '*SHA-256 mismatch; extraction was not attempted*'
        Assert-Equal 0 @(Get-ChildItem $fixture.Root -Directory).Count 'No extraction or output directories may be created.'
    }
    finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
}

foreach ($missing in @('ffmpeg', 'ffprobe', 'LICENSE', 'README.txt')) {
    Invoke-TestCase "missing archive $missing fails atomically" {
        $fixture = New-MediaFixture
        try {
            $fixture.Missing = if ($missing -in @('ffmpeg', 'ffprobe')) { $fixture.Manifest.executables[$missing] } else { "ffmpeg-7.0.2-essentials_build/$missing" }
            Write-MediaFixture $fixture
            Assert-Throws { Invoke-FixtureStage } '*Required archive file missing*'
            Assert-Equal 0 @(Get-ChildItem $fixture.Root -Directory).Count 'Failed staging must clean its scratch and not publish.'
        }
        finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
    }
}

foreach ($tool in @('ffmpeg', 'ffprobe')) {
    Invoke-TestCase "wrong $tool executable version fails" {
        $fixture = New-MediaFixture
        try {
            $fixture.VersionFailure = $tool
            Write-MediaFixture $fixture
            Assert-Throws { Invoke-FixtureStage } "*Unexpected $tool version*"
        }
        finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
    }
}

foreach ($encoder in @('libx264', 'libx265', 'h264_nvenc', 'hevc_nvenc')) {
    Invoke-TestCase "missing $encoder in staged inventory fails" {
        $fixture = New-MediaFixture
        try {
            $fixture.MissingEncoder = $encoder
            Write-MediaFixture $fixture
            Assert-Throws { Invoke-FixtureStage } "*lacks required encoder: $encoder*"
            $fixture.Manifest.requiredEncoders = @($fixture.Manifest.requiredEncoders | Where-Object { $_ -ne $encoder })
            $fixture.Manifest | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $fixture.Root 'manifest.json')
            Assert-Throws { Read-MediaToolsManifest (Join-Path $fixture.Root 'manifest.json') } "*must require encoder $encoder*"
        }
        finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
    }
}

Invoke-TestCase 'native version command failure retains diagnostics and prevents publication' {
    $fixture = New-MediaFixture
    try {
        $fixture.NativeFailure = $true
        Write-MediaFixture $fixture
        Assert-Throws { Invoke-FixtureStage } '*fixture failure*'
        Assert-True (-not (Test-Path (Join-Path $fixture.Root 'output'))) 'No output on native failure.'
        $pwsh = (Get-Process -Id $PID).Path
        Assert-Throws { & $nativeRunner $pwsh @('-NoProfile', '-Command', '[Console]::Error.WriteLine("native diagnostic"); exit 17') } '*failed (17)*native diagnostic*'
        Assert-Contains (& $nativeRunner $pwsh @('-NoProfile', '-Command', '[Console]::Write("success")')) 'success' 'Native success output must be captured.'
    }
    finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
}

foreach ($extra in @('../escape.txt', 'unrelated/FFPLAY.exe')) {
    Invoke-TestCase "unsafe or duplicate ZIP entry $extra fails" {
        $fixture = New-MediaFixture
        try {
            $fixture.Extra = $extra
            Write-MediaFixture $fixture
            Assert-Throws { Invoke-FixtureStage } '*archive*'
        }
        finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
    }
}

Invoke-TestCase 'download checksum failure never enters cache or extraction' {
    $fixture = New-MediaFixture
    try {
        $fixture.Manifest | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $fixture.Root 'manifest.json')
        function Invoke-WebRequest {
            param($Uri, $OutFile, $TimeoutSec)
            Assert-Equal $fixture.Manifest.archive.url $Uri 'Download must use pinned URL.'
            [IO.File]::WriteAllText($OutFile, 'invalid download')
        }
        Assert-Throws {
            Publish-MediaTools -ManifestPath (Join-Path $fixture.Root 'manifest.json') -CachePath $fixture.Root -OutputPath (Join-Path $fixture.Root 'output')
        } '*Downloaded archive SHA-256 mismatch*'
        Assert-Equal @('manifest.json') @(Get-ChildItem $fixture.Root -Force | ForEach-Object { $_.Name }) 'Failed download must leave no cache, scratch or output.'
    }
    finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
}

Invoke-TestCase 'encoder mentioned only in a description is not accepted' {
    $fixture = New-MediaFixture
    try {
        Write-MediaFixture $fixture
        function Invoke-MediaTool {
            param($Path, $Arguments)
            if ($Arguments -contains '-version') {
                return "$([IO.Path]::GetFileNameWithoutExtension($Path)) version $($fixture.Manifest.buildIdentity) Copyright"
            }
            return ' V....D other Description mentions libx264 libx265 h264_nvenc hevc_nvenc '
        }
        Assert-Throws { Invoke-FixtureStage } '*lacks required encoder: libx264*'
    }
    finally { Remove-Item -LiteralPath $fixture.Root -Recurse -Force }
}

Complete-TestRun
