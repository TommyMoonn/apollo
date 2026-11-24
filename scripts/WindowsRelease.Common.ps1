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

    Assert-TransferUnlinked $OutputPath
    Assert-TransferUnlinked $ExecutablePath
    if (Test-PathAtOrBelow -Path $OutputPath -Parent (Join-Path $ProjectRoot '.git')) {
        throw "Refusing to stage inside Git metadata: $OutputPath"
    }
    if (Test-Path -LiteralPath $OutputPath -PathType Leaf) {
        throw "Release staging output must be a directory: $OutputPath"
    }
    if (Test-PathAtOrBelow -Path $OutputPath -Parent $ProjectRoot) {
        $gitRoot = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('rev-parse','--show-toplevel') -AllowFailure
        if ($gitRoot.ExitCode -eq 0) {
            $relative = [IO.Path]::GetRelativePath($gitRoot.StdOut.Trim(), $OutputPath).Replace('\', '/')
            $tracked = (Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('--literal-pathspecs','ls-files','--',$relative)).StdOut
            if ($tracked) { throw 'Refusing to replace tracked release output.' }
        }
    }
    if (Test-Path -LiteralPath $OutputPath) { Assert-ReleaseDirectoryContents $OutputPath }
}

# Staging and verification share the same names and payload inventory.
function Get-WindowsPublicAssetNames { @('Apollo-Setup-x64.exe', 'Apollo-windows-x64.zip', 'SHA256SUMS.txt') }
function Get-WindowsArtifactNames { @(Get-WindowsPublicAssetNames) + @('release-provenance.json') }

function Get-WindowsPayloadPaths {
    param($MediaManifest)
    @('Apollo.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md') + @(Get-MediaToolPayloadPaths $MediaManifest)
}

function Get-WindowsPayloadSources {
    param([string]$ProjectRoot, [string]$ExecutablePath, [string]$MediaToolsPath, $MediaManifest)
    $sources = [ordered]@{}
    foreach ($name in (Get-WindowsPayloadPaths $MediaManifest)) {
        $sources[$name] = switch -CaseSensitive ($name) {
            'Apollo.exe' { $ExecutablePath }
            'LICENSE' { Join-Path $ProjectRoot $name }
            'THIRD_PARTY_NOTICES.md' { Join-Path $ProjectRoot $name }
            default { Join-Path $MediaToolsPath $name }
        }
    }
    return $sources
}

function Get-ReleaseFileRecord {
    param([string]$Path)
    Assert-TransferUnlinked $Path
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or $item.Length -le 0) { throw "Expected a nonempty regular file: $Path" }
    return [ordered]@{ sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant(); size = $item.Length }
}

function Assert-ReleaseDirectoryContents {
    param([string]$Directory, [switch]$Exact)
    Assert-TransferUnlinked $Directory
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { throw "Release artifact directory does not exist: $Directory" }
    $names = @(Get-WindowsArtifactNames)
    $items = @(Get-ChildItem -LiteralPath $Directory -Force)
    foreach ($item in $items) {
        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $names -cnotcontains $item.Name) {
            throw "Unexpected release output content: $($item.FullName)"
        }
        if ($Exact -and $item.Length -le 0) { throw "Empty release artifact: $($item.Name)" }
    }
    if ($Exact -and $items.Count -ne $names.Count) { throw 'Release artifact set is incomplete.' }
}

function Remove-ReleaseOutputDirectory {
    param([string]$Directory)
    Assert-ReleaseDirectoryContents $Directory
    foreach ($item in (Get-ChildItem -LiteralPath $Directory -Force)) { Remove-Item -LiteralPath $item.FullName -Force }
    Remove-Item -LiteralPath $Directory -Force
}

function Assert-ReleaseCandidateCheckout {
    param([string]$ProjectRoot, [string]$ExpectedSha, [switch]$IncludeUntracked)
    if ($ExpectedSha -cnotmatch '\A[0-9a-f]{40}\z') { throw 'Expected SHA must be a full lowercase Git commit ID.' }
    $root = (Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('rev-parse', '--show-toplevel')).StdOut.Trim()
    if (-not ([IO.Path]::GetFullPath($root).Equals($ProjectRoot, (Get-RepositoryPathComparison)))) { throw 'Candidate must be a Git repository root.' }
    $head = (Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('rev-parse', 'HEAD')).StdOut.Trim()
    if ($head -cne $ExpectedSha) { throw 'Candidate checkout does not match expected SHA.' }
    $untracked = if ($IncludeUntracked) { 'all' } else { 'no' }
    $status = (Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('status', '--porcelain=v1', "--untracked-files=$untracked")).StdOut
    if ($status) { throw 'Candidate checkout must be clean.' }
}

function Get-ReleaseRustPin {
    param([string]$ProjectRoot)
    $source = [IO.File]::ReadAllText((Join-Path $ProjectRoot 'rust-toolchain.toml'))
    $matches = [regex]::Matches($source, '(?m)^\s*channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$')
    if ($matches.Count -ne 1) { throw 'Expected one pinned Rust version.' }
    return $matches[0].Groups[1].Value
}

function Assert-ApolloResourceVersion {
    param([string]$Path, [string]$Version)
    Assert-WindowsX64ApolloExecutable -Path $Path
    $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
    $parts = "$($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart).$($info.FilePrivatePart)"
    $productParts = "$($info.ProductMajorPart).$($info.ProductMinorPart).$($info.ProductBuildPart).$($info.ProductPrivatePart)"
    if ($parts -cne "$Version.0" -or $productParts -cne "$Version.0" -or
        $info.FileVersion -cne $Version -or $info.ProductVersion -cne $Version -or $info.ProductName -cne 'Apollo' -or $info.OriginalFilename -cne 'Apollo.exe') {
        throw "Apollo executable resource identity does not match version $Version."
    }
}
