#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
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
