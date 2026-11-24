#requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'verify-windows-release.ps1')
. (Join-Path $PSScriptRoot '../tests/scripts/WindowsRelease.Smoke.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Default = (Join-Path $PSScriptRoot '..'); Aliases = @('-p','-ProjectRoot') }
    'artifacts-dir' = @{ Default = 'artifacts/windows' }
    'expected-version' = @{}
    'expected-sha' = @{}
    local = @{ Kind = 'Switch' }
    'discovery-harness' = @{}
    'require-nvenc' = @{ Kind = 'Switch' }
    'upgrade-only' = @{ Kind = 'Switch' }
    'media-only' = @{ Kind = 'Switch' }
    'media-dir' = @{}
}
if ($cli.help) {
    Write-CliHelp @'
Usage: smoke-windows-installer.ps1 --expected-version X.Y.Z (--local | --expected-sha SHA) [options]
       smoke-windows-installer.ps1 --media-only [--media-dir PATH] [--require-nvenc]

  --artifacts-dir PATH       Verified artifacts (default artifacts/windows).
  --project PATH             Matching checkout; relative paths resolve here.
  --discovery-harness PATH   Reuse a prebuilt discovery test executable.
  --require-nvenc            Require real NVIDIA encoding success.
  --upgrade-only             Require a strictly older published predecessor.
  --media-only               Run only the real media harness.
  --media-dir PATH           Select a staged pair (default .scratch/dev-media).
  --help                     Show help.

Installer acceptance requires a clean Windows x64 user or VM and access to
published Apollo releases via gh. Media-only mode does not install anything.
'@
    return
}
$root = Resolve-RepositoryRoot $cli.project
$options = @{ ProjectRoot = $root; RequireNvenc = $cli['require-nvenc']; DiscoveryHarness = $cli['discovery-harness'] }
if ($cli['media-only'] -or $cli['media-dir']) {
    if ($cli.local -or $cli['expected-sha'] -or $cli['expected-version'] -or $cli['upgrade-only']) { throw 'Media-only mode cannot be combined with installer identity or upgrade flags.' }
    $media = if ($cli['media-dir']) { $cli['media-dir'] } else { '.scratch/dev-media' }
    $options.MediaDirectory = Resolve-ReleasePath $root $media '--media-dir'
} else {
    $options.ReleaseDirectory = Resolve-ReleasePath $root $cli['artifacts-dir'] '--artifacts-dir'
    $options.ExpectedVersion = $cli['expected-version']
    $options.ExpectedSha = $cli['expected-sha']
    $options.Local = $cli.local
    $options.UpgradeOnly = $cli['upgrade-only']
}
Invoke-WindowsReleaseSmoke @options
