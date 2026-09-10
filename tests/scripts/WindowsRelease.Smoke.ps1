#requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$ReleaseDirectory,
    [Parameter(Mandatory)][string]$DiscoveryHarness
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows release smoke requires Windows.' }
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
. (Join-Path $projectRoot 'scripts/stage-media-tools.ps1')
$release = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$harness = (Resolve-Path -LiteralPath $DiscoveryHarness).Path
$manifestPath = Join-Path $projectRoot 'distribution/media-tools.json'
$manifest = Read-MediaToolsManifest $manifestPath
$registryPaths = @(
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Apollo',
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\Apollo.exe'
)
$shortcuts = @(
    (Join-Path ([Environment]::GetFolderPath('Programs')) 'Apollo.lnk'),
    (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Apollo.lnk')
)
foreach ($path in ($registryPaths + $shortcuts)) {
    if (Test-Path -LiteralPath $path) { throw "Release smoke requires a clean Apollo installation state; existing path: $path" }
}

$checksumNames = [Collections.Generic.List[string]]::new()
foreach ($line in (Get-Content (Join-Path $release 'SHA256SUMS.txt'))) {
    if ($line -cnotmatch '^([0-9a-f]{64})  (Apollo-Setup-x64\.exe|Apollo-windows-x64\.zip)$') { throw 'Unexpected checksum coverage.' }
    $checksumNames.Add($Matches[2])
    if ((Get-FileHash -LiteralPath (Join-Path $release $Matches[2])).Hash.ToLowerInvariant() -cne $Matches[1]) { throw "Release checksum mismatch: $line" }
}
if ($checksumNames.Count -ne 2 -or @($checksumNames | Sort-Object -Unique).Count -ne 2) { throw 'Expected each artifact checksum exactly once.' }

$scratch = Join-Path $projectRoot ('.scratch/release-smoke-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$portable = Join-Path $scratch 'portable'
$installed = Join-Path $scratch 'installed'

function Invoke-SmokeProcess {
    param([string]$Executable, [string]$RawArguments, [switch]$Discovery)
    $info = [Diagnostics.ProcessStartInfo]::new($Executable)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.WorkingDirectory = $scratch
    $info.Arguments = $RawArguments
    $info.Environment['PATH'] = ''
    if ($Discovery) {
        $info.Environment['APOLLO_DISCOVERY_TEST_MODE'] = 'bundle'
        [void]$info.Environment.Remove('APOLLO_TEST_REQUIRE_NVENC')
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        [void]$process.Start()
        if ($Discovery) {
            $stdout = $process.StandardOutput.ReadToEndAsync()
            $stderr = $process.StandardError.ReadToEndAsync()
        }
        if (-not $process.WaitForExit(60000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "Smoke process timed out: $Executable"
        }
        $diagnostic = if ($Discovery) { $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult() } else { '' }
        if ($process.ExitCode -ne 0) { throw "Smoke process failed ($($process.ExitCode)): $Executable`n$diagnostic" }
        if ($Discovery) {
            if ($diagnostic -notmatch 'Resolved .*ffmpeg.exe:' -or $diagnostic -notmatch '1 passed') { throw "Discovery harness did not run: $diagnostic" }
            Write-Host $diagnostic.Trim()
        }
    }
    finally { $process.Dispose() }
}

function Assert-Payload {
    param([string]$Directory)
    $expected = @('Apollo.exe','LICENSE','THIRD_PARTY_NOTICES.md') + @(Get-MediaToolPayloadPaths $manifest)
    $actual = @(Get-ChildItem -LiteralPath $Directory -Recurse -File | ForEach-Object {
        [IO.Path]::GetRelativePath($Directory, $_.FullName).Replace('\','/')
    } | Where-Object { $_ -ne 'Uninstall.exe' })
    if (@(Compare-Object ($expected | Sort-Object) ($actual | Sort-Object)).Count -ne 0) { throw "Unexpected payload in $Directory" }
    foreach ($name in $expected) {
        $reference = switch ($name) {
            'Apollo.exe' { Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release/apollo.exe' }
            'LICENSE' { Join-Path $projectRoot 'LICENSE' }
            'THIRD_PARTY_NOTICES.md' { Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md' }
            'licenses/ffmpeg/media-tools.json' { $manifestPath }
            default { Join-Path $scratch "reference/$name" }
        }
        if ((Get-FileHash -LiteralPath (Join-Path $Directory $name)).Hash -ne (Get-FileHash -LiteralPath $reference).Hash) { throw "Payload byte mismatch: $name" }
    }
    $probe = Join-Path $Directory 'discovery-test.exe'
    Copy-Item -LiteralPath $harness -Destination $probe
    try { Invoke-SmokeProcess $probe '--exact discovery_subprocess --nocapture' -Discovery }
    finally { Remove-Item -LiteralPath $probe -Force }
}

try {
    Publish-MediaTools -ManifestPath $manifestPath -CachePath (Join-Path $projectRoot '.scratch/media-tools-cache') -OutputPath (Join-Path $scratch 'reference') -Offline | Out-Null
    [IO.Compression.ZipFile]::ExtractToDirectory((Join-Path $release 'Apollo-windows-x64.zip'), $portable)
    Assert-Payload $portable
    # NSIS requires /D= to be the final, unquoted command-line argument.
    Invoke-SmokeProcess (Join-Path $release 'Apollo-Setup-x64.exe') ("/S /D=$installed")
    Assert-Payload $installed
    $location = (Get-ItemProperty -LiteralPath $registryPaths[0]).InstallLocation
    if ($location -ne $installed) { throw "Unexpected registered installation: $location" }
    'keep user file' | Set-Content (Join-Path $installed 'keep.txt')
    'keep license-adjacent user file' | Set-Content (Join-Path $installed 'licenses/ffmpeg/keep.txt')
    Invoke-SmokeProcess (Join-Path $installed 'Uninstall.exe') ("/S _?=$installed")
    foreach ($name in (@('Apollo.exe','LICENSE','THIRD_PARTY_NOTICES.md') + @(Get-MediaToolPayloadPaths $manifest))) {
        if (Test-Path -LiteralPath (Join-Path $installed $name)) { throw "Uninstall retained owned file: $name" }
    }
    foreach ($path in ($registryPaths + $shortcuts)) {
        if (Test-Path -LiteralPath $path) { throw "Uninstall retained registration or shortcut: $path" }
    }
    foreach ($name in @('keep.txt','licenses/ffmpeg/keep.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $installed $name))) { throw "Uninstall removed user file: $name" }
    }
    Write-Host 'Windows release smoke passed: checksums, exact payload bytes, no-PATH production discovery, install, uninstall, and user-file preservation.'
}
finally {
    # All of these paths were absent before this isolated smoke and belong to this run.
    foreach ($path in $registryPaths) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
    }
    foreach ($path in $shortcuts) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    Remove-Item -LiteralPath $scratch -Recurse -Force
}
