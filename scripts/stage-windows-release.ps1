#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'build-windows-installer.ps1')

function New-WindowsReleaseZip {
    param(
        [Parameter(Mandatory)]
        [string]$ZipPath,

        [Parameter(Mandatory)]
        [string]$ExecutablePath,

        [Parameter(Mandatory)]
        [string]$LicensePath,

        [Parameter(Mandatory)]
        [string]$NoticesPath,
        [string]$MediaToolsPath,
        $MediaManifest
    )

    Add-Type -AssemblyName System.IO.Compression

    $entries = @(
        [pscustomobject]@{ Name = 'Apollo.exe'; Source = $ExecutablePath },
        [pscustomobject]@{ Name = 'LICENSE'; Source = $LicensePath },
        [pscustomobject]@{ Name = 'THIRD_PARTY_NOTICES.md'; Source = $NoticesPath }
    )
    $entries += @(Get-MediaToolPayloadPaths $MediaManifest | ForEach-Object {
        [pscustomobject]@{ Name = $_; Source = (Join-Path $MediaToolsPath $_) }
    })

    $stream = [System.IO.File]::Open($ZipPath, [System.IO.FileMode]::CreateNew)
    try {
        $archive = [System.IO.Compression.ZipArchive]::new(
            $stream,
            [System.IO.Compression.ZipArchiveMode]::Create,
            $false
        )
        try {
            foreach ($item in $entries) {
                $entry = $archive.CreateEntry($item.Name, [System.IO.Compression.CompressionLevel]::Optimal)
                $entry.LastWriteTime = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
                $entryStream = $entry.Open()
                try {
                    $sourceStream = [System.IO.File]::OpenRead($item.Source)
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

    & (Join-Path $PSScriptRoot 'check-release.ps1') --project $projectRoot

    Assert-WindowsX64ApolloExecutable -Path $executablePath
    Assert-SafeStagingDirectory -ProjectRoot $projectRoot -ExecutablePath $executablePath -OutputPath $outputPath
    $cacheValue = if ($null -eq $cli['media-cache']) { '.scratch/media-tools-cache' } else { [string]$cli['media-cache'] }
    $cachePath = Resolve-ReleasePath -ProjectRoot $projectRoot -Path $cacheValue -OptionName '--media-cache'
    if (Test-PathAtOrBelow -Path $cachePath -Parent $outputPath) {
        throw 'Media cache must be outside the release staging output.'
    }

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
        $mediaPath = Publish-MediaTools -ManifestPath $manifestPath -CachePath $cachePath -OutputPath (Join-Path $temporaryOutput 'media-tools') -Offline:$cli['offline']
        $installerName = 'Apollo-Setup-x64.exe'
        $zipName = 'Apollo-windows-x64.zip'
        $installerPath = Join-Path $temporaryOutput $installerName
        $zipPath = Join-Path $temporaryOutput $zipName
        $checksumPath = Join-Path $temporaryOutput 'SHA256SUMS.txt'

        $installerArguments = @(
            '--project', $projectRoot,
            '--executable', $executablePath,
            '--output', $installerPath
        )
        if ($null -ne $cli['makensis']) {
            $installerArguments += @('--makensis', [string]$cli['makensis'])
        }
        Invoke-BuildWindowsInstallerCli -Arguments $installerArguments -MediaToolsPath $mediaPath

        if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
            throw "Installer build did not produce the expected staged artifact: $installerPath"
        }

        New-WindowsReleaseZip `
            -ZipPath $zipPath `
            -ExecutablePath $executablePath `
            -LicensePath $licensePath `
            -NoticesPath $noticesPath `
            -MediaToolsPath $mediaPath `
            -MediaManifest $mediaManifest

        Remove-Item -LiteralPath $mediaPath -Recurse -Force

        $checksumLines = foreach ($name in @($installerName, $zipName)) {
            $artifactPath = Join-Path $temporaryOutput $name
            $hash = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
            "$hash  $name"
        }
        [System.IO.File]::WriteAllText(
            $checksumPath,
            (($checksumLines -join "`n") + "`n"),
            [System.Text.UTF8Encoding]::new($false)
        )

        $backupPath = Join-Path $outputParent (".$outputName.previous-" + [Guid]::NewGuid().ToString('N'))
        $hadOutput = Test-Path -LiteralPath $outputPath
        if ($hadOutput) { [IO.Directory]::Move($outputPath, $backupPath) }
        try { [IO.Directory]::Move($temporaryOutput, $outputPath) }
        catch {
            if ($hadOutput) { [IO.Directory]::Move($backupPath, $outputPath) }
            throw
        }
        if ($hadOutput) { Remove-Item -LiteralPath $backupPath -Recurse -Force }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryOutput) {
            Remove-Item -LiteralPath $temporaryOutput -Recurse -Force
        }
    }

    Write-Host "Created:  $(Join-Path $outputPath 'Apollo-Setup-x64.exe')"
    Write-Host "Created:  $(Join-Path $outputPath 'Apollo-windows-x64.zip')"
    Write-Host "Checksum: $(Join-Path $outputPath 'SHA256SUMS.txt')"
}

if ($MyInvocation.InvocationName -ne '.') {
    Invoke-StageWindowsReleaseCli -Arguments $args
}
