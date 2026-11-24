#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
. (Join-Path $PSScriptRoot 'build-windows-installer.ps1')
. (Join-Path $PSScriptRoot 'verify-windows-release.ps1')

function New-WindowsReleaseZip {
    param(
        [Parameter(Mandatory)]
        [string]$ZipPath,

        [Parameter(Mandatory)]
        $PayloadSources
    )

    Add-Type -AssemblyName System.IO.Compression

    $stream = [System.IO.File]::Open($ZipPath, [System.IO.FileMode]::CreateNew)
    try {
        $archive = [System.IO.Compression.ZipArchive]::new(
            $stream,
            [System.IO.Compression.ZipArchiveMode]::Create,
            $false
        )
        try {
            foreach ($name in $PayloadSources.Keys) {
                $entry = $archive.CreateEntry($name, [System.IO.Compression.CompressionLevel]::Optimal)
                $entry.LastWriteTime = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
                $entryStream = $entry.Open()
                try {
                    $sourceStream = [System.IO.File]::OpenRead($PayloadSources[$name])
                    try {
                        $sourceStream.CopyTo($entryStream)
                    }
                    finally {
                        $sourceStream.Dispose()
                    }
                }
                finally {
                    $entryStream.Dispose()
                }
            }
        }
        finally {
            $archive.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Invoke-StageWindowsReleaseCli {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Arguments
    )

    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
        executable = @{ Aliases = @('-e', '-Executable') }
        makensis = @{ Aliases = @('-m', '-Makensis') }
        output = @{ Aliases = @('-o', '-Output') }
        'media-cache' = @{}
        offline = @{ Kind = 'Switch' }
        'expected-sha' = @{}
        'expected-version' = @{}
    }

    if ($cli['help']) {
        Write-CliHelp @'
Usage: .\scripts\stage-windows-release.ps1 [options]

Options:
  -p, --project <path>          Project root. Defaults to this repository.
  -e, --executable <path>       Built apollo.exe. Defaults to the Windows x64 release target.
  -m, --makensis <path>         NSIS makensis compiler. Defaults to installer-build discovery.
  -o, --output <path>           Staging directory. Defaults to artifacts/windows.
  -h, --help                    Show this help.
  --media-cache <path>          Pinned archive cache. Defaults to .scratch/media-tools-cache.
  --offline                    Require a valid cached media archive.
  --expected-sha <sha>         Require an exact clean candidate checkout.
  --expected-version <version> Required with --expected-sha; exact Cargo version.

Without expected identity the staged output is local-only.

Relative executable, compiler, and output paths resolve from the project root.
'@
        return
    }

    $projectRoot = Resolve-RepositoryRoot -Path ([string]$cli['project'])
    $defaultExecutable = 'target/x86_64-pc-windows-msvc/release/apollo.exe'
    $defaultOutput = 'artifacts/windows'
    $executableValue = if ($null -eq $cli['executable']) { $defaultExecutable } else { [string]$cli['executable'] }
    $outputValue = if ($null -eq $cli['output']) { $defaultOutput } else { [string]$cli['output'] }
    $executablePath = Resolve-ReleasePath -ProjectRoot $projectRoot -Path $executableValue -OptionName '--executable'
    $outputPath = Resolve-ReleasePath -ProjectRoot $projectRoot -Path $outputValue -OptionName '--output'

    $release = Invoke-ReleaseValidation -ProjectRoot $projectRoot
    $expectedSha = $cli['expected-sha']
    $expectedVersion = $cli['expected-version']
    if (($null -eq $expectedSha) -ne ($null -eq $expectedVersion)) { throw 'Supply both --expected-sha and --expected-version, or neither.' }
    if ($null -ne $expectedSha) {
        Assert-ReleaseCandidateCheckout -ProjectRoot $projectRoot -ExpectedSha $expectedSha -IncludeUntracked
        if ($expectedVersion -cne $release.Version) { throw 'Candidate version does not match Cargo metadata.' }
    }
    Assert-ApolloResourceVersion -Path $executablePath -Version $release.Version
    Assert-SafeStagingDirectory -ProjectRoot $projectRoot -ExecutablePath $executablePath -OutputPath $outputPath
    $cacheValue = if ($null -eq $cli['media-cache']) { '.scratch/media-tools-cache' } else { [string]$cli['media-cache'] }
    $cachePath = Resolve-ReleasePath -ProjectRoot $projectRoot -Path $cacheValue -OptionName '--media-cache'
    Assert-TransferUnlinked $cachePath
    if ((Test-PathAtOrBelow -Path $cachePath -Parent $outputPath) -or (Test-PathAtOrBelow -Path $outputPath -Parent $cachePath)) {
        throw 'Media cache must be outside the release staging output.'
    }

    $compilerPath = Resolve-NsisCompilerPath -ProjectRoot $projectRoot -Value $cli['makensis']
    if (Test-PathAtOrBelow -Path $compilerPath -Parent $outputPath) { throw 'Output must not contain the NSIS compiler.' }
    Push-Location $projectRoot
    try {
        $rustOutput = & rustc --version
        if ($LASTEXITCODE -ne 0 -or "$rustOutput" -cnotmatch '^rustc ([0-9]+\.[0-9]+\.[0-9]+) ') { throw 'Could not identify selected Rust compiler.' }
        $rustVersion = $Matches[1]
        if ($rustVersion -cne (Get-ReleaseRustPin $projectRoot)) { throw 'Selected Rust version does not match the project pin.' }
        $nsisOutput = & $compilerPath /VERSION
        if ($LASTEXITCODE -ne 0 -or "$nsisOutput".Trim() -cne 'v3.11') { throw 'Release staging requires NSIS v3.11.' }
    } finally { Pop-Location }

    $licensePath = Join-Path $projectRoot 'LICENSE'
    $noticesPath = Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md'
    if (-not (Test-Path -LiteralPath $licensePath -PathType Leaf)) {
        throw "Project license does not exist: $licensePath"
    }
    if (-not (Test-Path -LiteralPath $noticesPath -PathType Leaf)) {
        throw "Third-party notices file does not exist: $noticesPath"
    }

    $outputParent = Split-Path $outputPath -Parent
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
    $outputName = Split-Path $outputPath -Leaf
    $temporaryOutput = Join-Path $outputParent (".$outputName.stage-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temporaryOutput | Out-Null

    try {
        $manifestPath = Join-Path $projectRoot 'distribution/media-tools.json'
        $mediaManifest = Read-MediaToolsManifest $manifestPath
        $mediaPath = Publish-MediaTools -ManifestPath $manifestPath -Manifest $mediaManifest -CachePath $cachePath -OutputPath (Join-Path $temporaryOutput 'media-tools') -Offline:$cli['offline']
        $payloadSources = Get-WindowsPayloadSources -ProjectRoot $projectRoot -ExecutablePath $executablePath -MediaToolsPath $mediaPath -MediaManifest $mediaManifest
        $payload = [ordered]@{}
        foreach ($name in $payloadSources.Keys) { $payload[$name] = Get-ReleaseFileRecord $payloadSources[$name] }
        $installerName = 'Apollo-Setup-x64.exe'
        $zipName = 'Apollo-windows-x64.zip'
        $installerPath = Join-Path $temporaryOutput $installerName
        $zipPath = Join-Path $temporaryOutput $zipName
        $checksumPath = Join-Path $temporaryOutput 'SHA256SUMS.txt'

        $installerArguments = @(
            '--project', $projectRoot,
            '--executable', $executablePath,
            '--output', $installerPath,
            '--makensis', $compilerPath
        )
        Invoke-BuildWindowsInstallerCli -Arguments $installerArguments -MediaToolsPath $mediaPath -Release $release -MediaManifest $mediaManifest

        if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
            throw "Installer build did not produce the expected staged artifact: $installerPath"
        }

        New-WindowsReleaseZip -ZipPath $zipPath -PayloadSources $payloadSources

        Remove-Item -LiteralPath $mediaPath -Recurse -Force

        $assets = [ordered]@{}
        $checksumLines = foreach ($name in @($installerName, $zipName)) {
            $assets[$name] = Get-ReleaseFileRecord (Join-Path $temporaryOutput $name)
            "$($assets[$name].sha256)  $name"
        }
        [IO.File]::WriteAllText($checksumPath, (($checksumLines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
        $assets['SHA256SUMS.txt'] = Get-ReleaseFileRecord $checksumPath
        $provenance = [ordered]@{
            schemaVersion = 1
            candidateSha = $expectedSha
            version = $release.Version
            target = 'x86_64-pc-windows-msvc'
            rustVersion = $rustVersion
            nsisVersion = "$nsisOutput".Trim().Substring(1)
            mediaArchive = [ordered]@{ filename = $mediaManifest.archive.filename; url = $mediaManifest.archive.url; sha256 = $mediaManifest.archive.sha256; buildIdentity = $mediaManifest.buildIdentity }
            assets = $assets
            payload = $payload
        }
        [IO.File]::WriteAllText((Join-Path $temporaryOutput 'release-provenance.json'), (($provenance | ConvertTo-Json -Depth 8) + "`n"), [Text.UTF8Encoding]::new($false))
        if ($null -ne $expectedSha) { Assert-ReleaseCandidateCheckout -ProjectRoot $projectRoot -ExpectedSha $expectedSha }
        Assert-WindowsReleaseArtifacts -ProjectRoot $projectRoot -ArtifactsDirectory $temporaryOutput -ExpectedVersion $release.Version -ExpectedSha $expectedSha -Local:($null -eq $expectedSha)

        $backupPath = Join-Path $outputParent (".$outputName.previous-" + [Guid]::NewGuid().ToString('N'))
        $hadOutput = Test-Path -LiteralPath $outputPath
        if ($hadOutput) {
            Assert-ReleaseDirectoryContents $outputPath
            [IO.Directory]::Move($outputPath, $backupPath)
        }
        try { [IO.Directory]::Move($temporaryOutput, $outputPath) }
        catch {
            if ($hadOutput) { [IO.Directory]::Move($backupPath, $outputPath) }
            throw
        }
        if ($hadOutput) { Remove-ReleaseOutputDirectory $backupPath }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryOutput) {
            Remove-Item -LiteralPath $temporaryOutput -Recurse -Force
        }
    }

    Write-Host "Created:  $(Join-Path $outputPath 'Apollo-Setup-x64.exe')"
    Write-Host "Created:  $(Join-Path $outputPath 'Apollo-windows-x64.zip')"
    Write-Host "Provenance: $(Join-Path $outputPath 'release-provenance.json')"
    Write-Host "Checksum: $(Join-Path $outputPath 'SHA256SUMS.txt')"
}

if ($MyInvocation.InvocationName -ne '.') {
    Invoke-StageWindowsReleaseCli -Arguments $args
}
