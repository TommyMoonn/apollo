#requires -Version 7.0

# Adapted from Archeion scripts/coverage-rust.ps1 at 4b711fc2 (GPL-3.0-only).
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# Check native exit codes explicitly on every supported PowerShell version.
$PSNativeCommandUseErrorActionPreference = $false

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    install = @{ Kind = 'Switch' }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\coverage-rust.ps1 [options]

Options:
  --install    Install the pinned Rust coverage prerequisites, then exit.
  -h, --help   Show this help.

Without options, run the ordinary Windows x64 Rust tests under coverage
and write a terminal summary and HTML report under .scratch/coverage/.
'@
    return
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$target = 'x86_64-pc-windows-msvc'
$coverageVersion = '0.9.0'
$installHint = 'Run: pwsh -NoProfile -File scripts/coverage-rust.ps1 --install'
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
    throw "Coverage requires Windows x64 ($target)."
}
if (-not (Get-Command rustup -CommandType Application -ErrorAction SilentlyContinue)) {
    throw 'Rustup is required. Install Rustup for Windows x64 and add it to PATH.'
}

$channelMatches = @(Select-String -LiteralPath (Join-Path $repoRoot 'rust-toolchain.toml') -Pattern '^\s*channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$')
if ($channelMatches.Count -ne 1) {
    throw 'Expected one exact Rust version in rust-toolchain.toml.'
}
$channel = $channelMatches[0].Matches[0].Groups[1].Value
$toolchain = "$channel-$target"

Push-Location -LiteralPath $repoRoot
try {
    if ($cli['install']) {
        Write-CliStep -Message "Installing $toolchain with llvm-tools-preview"
        & rustup toolchain install $toolchain --profile minimal --component llvm-tools-preview
        if ($LASTEXITCODE -ne 0) { throw "Rust coverage toolchain installation failed (exit $LASTEXITCODE)." }

        Write-CliStep -Message "Installing cargo-llvm-cov $coverageVersion"
        & rustup run $toolchain cargo install cargo-llvm-cov --version "=$coverageVersion" --locked
        if ($LASTEXITCODE -ne 0) { throw "cargo-llvm-cov installation failed (exit $LASTEXITCODE)." }
        Write-CliSuccess -Message 'Coverage prerequisites installed.'
        return
    }

    # rustup run fails if the exact toolchain is missing; it does not install it.
    $rustVersion = @(& rustup run $toolchain rustc --version --verbose)
    if ($LASTEXITCODE -ne 0 -or $rustVersion -notcontains "host: $target" -or $rustVersion -notcontains "release: $channel") {
        throw "The pinned Rust toolchain $toolchain is required. $installHint"
    }
    $sysroot = & rustup run $toolchain rustc --print sysroot
    if ($LASTEXITCODE -ne 0) { throw "Unable to locate the pinned Rust sysroot. $installHint" }
    $llvmBin = Join-Path $sysroot "lib/rustlib/$target/bin"
    $llvmCov = Join-Path $llvmBin 'llvm-cov.exe'
    $llvmProfdata = Join-Path $llvmBin 'llvm-profdata.exe'
    if (-not (Test-Path -LiteralPath $llvmCov -PathType Leaf) -or -not (Test-Path -LiteralPath $llvmProfdata -PathType Leaf)) {
        throw "llvm-tools-preview for $toolchain is required. $installHint"
    }

    $installedVersion = & rustup run $toolchain cargo llvm-cov --version
    if ($LASTEXITCODE -ne 0 -or $installedVersion -cne "cargo-llvm-cov $coverageVersion") {
        throw "cargo-llvm-cov $coverageVersion is required (found: $installedVersion). $installHint"
    }

    $targetRoot = Join-Path $repoRoot 'target'
    $coverageTarget = Join-Path $targetRoot 'llvm-cov-target'
    $reportRoot = Join-Path $repoRoot '.scratch/coverage'
    # A new report directory avoids mixing stale HTML pages with a later run.
    $runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $reportDir = Join-Path $reportRoot $runId
    Assert-TransferUnlinked -Path $coverageTarget
    Assert-TransferUnlinked -Path $reportDir
    New-Item -ItemType Directory -Path $reportDir | Out-Null

    # Keep instrumentation and LLVM profiles under target/, even with a custom
    # Cargo target directory. Use the compiler's own LLVM tools, never PATH's.
    $coverageEnvironment = @{
        CARGO_TARGET_DIR = $targetRoot
        CARGO_LLVM_COV_TARGET_DIR = $coverageTarget
        CARGO_LLVM_COV_BUILD_DIR = $coverageTarget
        CARGO_LLVM_COV_SETUP = 'no'
        LLVM_COV = $llvmCov
        LLVM_PROFDATA = $llvmProfdata
    }
    $previousEnvironment = @{}
    foreach ($name in $coverageEnvironment.Keys) {
        $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }
    try {
        foreach ($name in $coverageEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $coverageEnvironment[$name], 'Process')
        }

        Write-CliHeading -Text 'Rust coverage diagnostics'
        Write-CliDetail -Label 'Target' -Value $target
        Write-CliDetail -Label 'Toolchain' -Value $channel
        Write-CliDetail -Label 'Coverage tool' -Value $coverageVersion
        Write-CliDetail -Label 'Tests' -Value 'All ordinary Rust targets; ignored real-media/discovery tests excluded'
        Write-CliDetail -Label 'Reports' -Value 'HTML and terminal summary'

        $manifest = Join-Path $repoRoot 'Cargo.toml'
        $cargoArgs = @('--locked', '--manifest-path', $manifest, '--target', $target)
        # The normal HTML invocation clears old coverage profiles before testing.
        # The report subcommand reuses the collected data without rerunning tests.
        & rustup run $toolchain cargo llvm-cov @cargoArgs --all-targets --html --output-dir $reportDir
        if ($LASTEXITCODE -ne 0) { throw "Rust coverage test/report failed (exit $LASTEXITCODE)." }

        $index = Join-Path $reportDir 'html/index.html'
        if (-not (Test-Path -LiteralPath $index -PathType Leaf)) {
            throw "Coverage did not produce the expected HTML report: $index"
        }
        & rustup run $toolchain cargo llvm-cov report @cargoArgs | Tee-Object -FilePath (Join-Path $reportDir 'summary.txt')
        if ($LASTEXITCODE -ne 0) { throw "Rust coverage summary failed (exit $LASTEXITCODE)." }

        Write-CliSuccess -Message 'Coverage diagnostics complete.'
        Write-CliDetail -Label 'HTML report' -Value $index -ValueTone 'Important'
    }
    finally {
        foreach ($name in $previousEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
        }
    }
}
finally {
    Pop-Location
}
