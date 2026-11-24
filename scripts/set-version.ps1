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
  -v, --version <version>       Stable X.Y.Z version to set. May also be positional.
  -p, --project <path>          Project root to update.
  -h, --help                    Show this help.
'@
    return
}

$version = [string]$cli['version']
if ([string]::IsNullOrWhiteSpace($version)) {
    throw "A stable release version is required. Run with --help to see usage."
}
Assert-StableReleaseVersion -Version $version

$projectRoot = Resolve-RepositoryRoot -Path ([string]$cli['project'])
$manifestPath = Join-Path $projectRoot 'Cargo.toml'
$lockPath = Join-Path $projectRoot 'Cargo.lock'
if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
    throw "Cargo lockfile does not exist: $lockPath"
}

$current = Invoke-ReleaseValidation -ProjectRoot $projectRoot
if ($current.Version -ceq $version) {
    Write-CliSuccess "Apollo is already at $version."
    return
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

$manifestText = [System.IO.File]::ReadAllText($manifestPath)
$lockText = [System.IO.File]::ReadAllText($lockPath)
$package = Get-CargoManifestPackageInfo -ManifestPath $manifestPath
$lockPackage = Get-CargoLockRootPackageInfo -LockPath $lockPath -PackageName $package.Name
$updatedManifest = $manifestText.Remove($package.VersionIndex, $package.VersionLength).Insert($package.VersionIndex, $version)
$expectedLock = $lockText.Remove($lockPackage.VersionIndex, $lockPackage.VersionLength).Insert($lockPackage.VersionIndex, $version)
$hasBom = $manifestBytes.Length -ge 3 -and $manifestBytes[0] -eq 0xEF -and $manifestBytes[1] -eq 0xBB -and $manifestBytes[2] -eq 0xBF

Write-CliHeading "Updating Apollo to $version"
Push-Location -LiteralPath $projectRoot
try {
    Write-CliStep -Message 'Updating Cargo package version'
    [System.IO.File]::WriteAllText($manifestPath, $updatedManifest, [System.Text.UTF8Encoding]::new($hasBom))

    Write-CliStep -Message 'Refreshing the root lock entry through Cargo'
    Invoke-VersionCargo -CargoArguments @(
        'update', '--manifest-path', $manifestPath,
        '-p', "$($package.Name)@$($lockPackage.Version)", '--precise', $version
    )
    Invoke-VersionCargo -CargoArguments @(
        'metadata', '--manifest-path', $manifestPath, '--locked', '--format-version', '1', '--no-deps'
    )

    # Cargo may normalize CRLF on a Windows checkout. No other lock change is allowed.
    $actualLock = [System.IO.File]::ReadAllText($lockPath)
    if (-not [string]::Equals($actualLock.Replace("`r`n", "`n"), $expectedLock.Replace("`r`n", "`n"), [System.StringComparison]::Ordinal)) {
        throw 'Cargo changed lockfile content beyond the Apollo root version.'
    }
    if (-not [string]::Equals([System.IO.File]::ReadAllText($manifestPath), $updatedManifest, [System.StringComparison]::Ordinal)) {
        throw 'Cargo changed the manifest unexpectedly.'
    }
    $null = Invoke-ReleaseValidation -ProjectRoot $projectRoot
}
catch {
    $updateFailure = $_
    $restoreFailures = [System.Collections.Generic.List[string]]::new()
    foreach ($backup in @(@{ Path = $manifestPath; Bytes = $manifestBytes }, @{ Path = $lockPath; Bytes = $lockBytes })) {
        try {
            [System.IO.File]::WriteAllBytes($backup.Path, $backup.Bytes)
        }
        catch {
            $restoreFailures.Add("$($backup.Path): $($_.Exception.Message)")
        }
    }
    if ($restoreFailures.Count -gt 0) {
        throw "Version update failed: $($updateFailure.Exception.Message). Backup restoration also failed: $($restoreFailures -join '; ')"
    }
    Write-CliWarning 'Version update failed. Original version files were restored.'
    throw $updateFailure
}
finally {
    Pop-Location
}

Write-CliSuccess "Updated Apollo release version to $version."
Write-CliDetail -Label 'Next' -Value 'Prepare the matching changelog and release note.'
