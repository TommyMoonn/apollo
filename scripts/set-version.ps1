#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'check-release.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
    version = @{ Aliases = @('-v', '-Version') }
} -Positionals @('version')

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\set-version.ps1 <version> [options]

Options:
  -v, --version <version>       Semantic version to set. May also be positional.
  -p, --project <path>          Project root to update.
  -h, --help                    Show this help.
'@
    return
}

$version = [string]$cli['version']
if ([string]::IsNullOrWhiteSpace($version)) {
    throw "A semantic version is required. Run with --help to see usage."
}
if (-not (Test-SemanticVersion -Version $version)) {
    throw "Version '$version' is not a valid semantic version."
}

$projectRoot = Resolve-RepositoryRoot -Path ([string]$cli['project'])
$manifestPath = Join-Path $projectRoot 'Cargo.toml'
$lockPath = Join-Path $projectRoot 'Cargo.lock'
if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
    throw "Cargo lockfile does not exist: $lockPath"
}

$manifestBytes = [System.IO.File]::ReadAllBytes($manifestPath)
$lockBytes = [System.IO.File]::ReadAllBytes($lockPath)

function Invoke-VersionCargo {
    param(
        [Parameter(Mandatory)]
        [string[]]$CargoArguments
    )

    $output = @(& cargo @CargoArguments 2>&1)
    if ($LASTEXITCODE -ne 0) {
        $details = ($output | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
        throw "Cargo failed: cargo $($CargoArguments -join ' ')`n$details"
    }
}

try {
    $manifestText = [System.IO.File]::ReadAllText($manifestPath)
    $package = Get-CargoManifestPackageInfo -ManifestPath $manifestPath
    $lockVersion = Get-CargoLockRootPackageVersion -LockPath $lockPath -PackageName $package.Name
    $updatedManifest = $manifestText.Remove($package.VersionIndex, $package.VersionLength).Insert($package.VersionIndex, $version)
    [System.IO.File]::WriteAllText($manifestPath, $updatedManifest, [System.Text.UTF8Encoding]::new($false))

    Invoke-VersionCargo -CargoArguments @(
        'update',
        '--manifest-path', $manifestPath,
        '-p', "$($package.Name)@$lockVersion",
        '--precise', $version
    )
    Invoke-VersionCargo -CargoArguments @(
        'metadata',
        '--manifest-path', $manifestPath,
        '--locked',
        '--format-version', '1',
        '--no-deps'
    )

    $null = Invoke-ReleaseValidation -ProjectRoot $projectRoot
}
catch {
    [System.IO.File]::WriteAllBytes($manifestPath, $manifestBytes)
    [System.IO.File]::WriteAllBytes($lockPath, $lockBytes)
    throw
}

Write-Host "Updated Apollo release version to $version."
