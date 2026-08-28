#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

function Resolve-ReleasePath {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$OptionName
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "Option '$OptionName' cannot be empty."
    }

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }

    return [System.IO.Path]::GetFullPath((Join-Path $ProjectRoot $Path))
}

function Test-PathAtOrBelow {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Parent
    )

    $comparison = Get-RepositoryPathComparison
    $separator = [System.IO.Path]::DirectorySeparatorChar
    $fullPath = [System.IO.Path]::GetFullPath($Path).TrimEnd([char[]]@('\', '/'))
    $fullParent = [System.IO.Path]::GetFullPath($Parent).TrimEnd([char[]]@('\', '/'))

    if ($fullPath.Equals($fullParent, $comparison)) {
        return $true
    }

    return $fullPath.StartsWith("$fullParent$separator", $comparison)
}

function Assert-WindowsX64ApolloExecutable {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Windows x64 Apollo release executable does not exist: $Path"
    }
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Release executable path must identify exactly one file: $Path"
    }

    $fileName = [System.IO.Path]::GetFileName($Path)
    if (-not $fileName.Equals('apollo.exe', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Release executable must be named 'apollo.exe', found '$fileName'."
    }

    $stream = [System.IO.File]::OpenRead($Path)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5A4D) {
            throw "Release executable is not a Windows PE file: $Path"
        }

        $stream.Position = 0x3C
        $peOffset = $reader.ReadInt32()
        if ($peOffset -lt 0 -or ([int64]$peOffset + 6) -gt $stream.Length) {
            throw "Release executable has an invalid Windows PE header: $Path"
        }

        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "Release executable is not a Windows PE file: $Path"
        }

        $machine = $reader.ReadUInt16()
        if ($machine -ne 0x8664) {
            throw ("Release executable must target Windows x64 (AMD64); PE machine is 0x{0:x4}." -f $machine)
        }

        if (([int64]$peOffset + 24) -gt $stream.Length) {
            throw "Release executable has a truncated Windows PE file header: $Path"
        }

        $stream.Position = $peOffset + 20
        $optionalHeaderSize = $reader.ReadUInt16()
        $optionalHeaderOffset = [int64]$peOffset + 24
        $minimumOptionalHeaderSize = 70
        if ($optionalHeaderSize -lt $minimumOptionalHeaderSize -or
            ($optionalHeaderOffset + $optionalHeaderSize) -gt $stream.Length) {
            throw "Release executable has a malformed Windows PE optional header: $Path"
        }

        $stream.Position = $optionalHeaderOffset
        $optionalHeaderMagic = $reader.ReadUInt16()
        if ($optionalHeaderMagic -ne 0x020B) {
            throw ("Release executable must use PE32+; optional-header magic is 0x{0:x4}." -f $optionalHeaderMagic)
        }

        $stream.Position = $optionalHeaderOffset + 68
        $subsystem = $reader.ReadUInt16()
        if ($subsystem -ne 2) {
            throw ("Release executable must use the Windows GUI subsystem; PE subsystem is {0}." -f $subsystem)
        }
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Assert-SafeStagingDirectory {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [Parameter(Mandatory)]
        [string]$ExecutablePath,

        [Parameter(Mandatory)]
        [string]$OutputPath
    )

    if (Test-RepositoryFileSystemRoot -Path $OutputPath) {
        throw "Refusing to use a filesystem root as the staging directory: $OutputPath"
    }

    if (Test-PathAtOrBelow -Path $ProjectRoot -Parent $OutputPath) {
        throw "Refusing to stage into the project root or one of its ancestors: $OutputPath"
    }

    if (Test-PathAtOrBelow -Path $ExecutablePath -Parent $OutputPath) {
        throw "Refusing to stage into a directory that contains the release executable: $OutputPath"
    }

    if (Test-Path -LiteralPath $OutputPath -PathType Leaf) {
        throw "Release staging output must be a directory: $OutputPath"
    }
}

function New-WindowsReleaseZip {
    param(
        [Parameter(Mandatory)]
        [string]$ZipPath,

        [Parameter(Mandatory)]
        [string]$ExecutablePath,

        [Parameter(Mandatory)]
        [string]$LicensePath,

        [Parameter(Mandatory)]
        [string]$NoticesPath
    )

    Add-Type -AssemblyName System.IO.Compression

    $entries = @(
        [pscustomobject]@{ Name = 'Apollo.exe'; Source = $ExecutablePath },
        [pscustomobject]@{ Name = 'LICENSE'; Source = $LicensePath },
        [pscustomobject]@{ Name = 'THIRD_PARTY_NOTICES.md'; Source = $NoticesPath }
    )

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

    $licensePath = Join-Path $projectRoot 'LICENSE'
    $noticesPath = Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md'
    if (-not (Test-Path -LiteralPath $licensePath -PathType Leaf)) {
        throw "Project license does not exist: $licensePath"
    }
    if (-not (Test-Path -LiteralPath $noticesPath -PathType Leaf)) {
        throw "Third-party notices do not exist: $noticesPath"
    }

    $outputParent = Split-Path $outputPath -Parent
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
    $outputName = Split-Path $outputPath -Leaf
    $temporaryOutput = Join-Path $outputParent (".$outputName.stage-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temporaryOutput | Out-Null

    try {
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
        & (Join-Path $PSScriptRoot 'build-windows-installer.ps1') @installerArguments

        if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
            throw "Installer build did not produce the expected staged artifact: $installerPath"
        }

        New-WindowsReleaseZip `
            -ZipPath $zipPath `
            -ExecutablePath $executablePath `
            -LicensePath $licensePath `
            -NoticesPath $noticesPath

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

        if (Test-Path -LiteralPath $outputPath) {
            Remove-Item -LiteralPath $outputPath -Recurse -Force
        }
        Move-Item -LiteralPath $temporaryOutput -Destination $outputPath
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
