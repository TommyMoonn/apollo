#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
. (Join-Path $PSScriptRoot "Test.Common.ps1")
. (Join-Path $projectRoot "scripts/Cli.Common.ps1")

$optionSpecs = @{
    project = @{ Aliases = @('-p', '-ProjectRoot') }
    output = @{ Aliases = @('-o', '-OutputPath') }
    'dry-run' = @{ Kind = 'Switch'; Aliases = @('-n', '-DryRun') }
}

Invoke-TestCase "converts integer CLI values" {
    $value = ConvertTo-CliInt -Value '400' -OptionName '--part-size-mib' -Minimum 64 -Maximum 440
    Assert-Equal 400 $value "Integer conversion mismatch."
}

Invoke-TestCase "rejects non-integer CLI values" {
    Assert-Throws {
        ConvertTo-CliInt -Value '400.5' -OptionName '--part-size-mib' -Minimum 64 -Maximum 440
    } "*expects an integer value*"
}

Invoke-TestCase "rejects CLI integers outside the allowed range" {
    Assert-Throws {
        ConvertTo-CliInt -Value '63' -OptionName '--part-size-mib' -Minimum 64 -Maximum 440
    } "*must be between 64 and 440*"

    Assert-Throws {
        ConvertTo-CliInt -Value '441' -OptionName '--part-size-mib' -Minimum 64 -Maximum 440
    } "*must be between 64 and 440*"
}

Invoke-TestCase "resolves CLI choices case-insensitively" {
    $value = Resolve-CliChoice -Value 'AUTO' -OptionName '--runtime' -AllowedValues @('Auto', 'Docker', 'WSL')
    Assert-Equal 'Auto' $value "Choice resolution should return the configured canonical value."
}

Invoke-TestCase "rejects unsupported CLI choices" {
    Assert-Throws {
        Resolve-CliChoice -Value 'Podman' -OptionName '--runtime' -AllowedValues @('Auto', 'Docker', 'WSL')
    } "*expects one of: Auto, Docker, WSL*"
}

Invoke-TestCase "parses canonical long option" {
    $cli = ConvertFrom-CliArguments -Arguments @('--project', 'C:\repo') -OptionSpecs $optionSpecs
    Assert-Equal 'C:\repo' $cli['project'] "Long option value mismatch."
}

Invoke-TestCase "parses short option" {
    $cli = ConvertFrom-CliArguments -Arguments @('-p', 'C:\repo') -OptionSpecs $optionSpecs
    Assert-Equal 'C:\repo' $cli['project'] "Short option value mismatch."
}

Invoke-TestCase "parses compatibility alias case-insensitively" {
    $cli = ConvertFrom-CliArguments -Arguments @('-ProjectRoot', 'C:\repo') -OptionSpecs $optionSpecs
    Assert-Equal 'C:\repo' $cli['project'] "Compatibility alias value mismatch."
}

Invoke-TestCase "parses attached long option value" {
    $cli = ConvertFrom-CliArguments -Arguments @('--output=result.zip') -OptionSpecs $optionSpecs
    Assert-Equal 'result.zip' $cli['output'] "Attached value mismatch."
}

Invoke-TestCase "parses switch" {
    $cli = ConvertFrom-CliArguments -Arguments @('--dry-run') -OptionSpecs $optionSpecs
    Assert-Equal $true $cli['dry-run'] "Switch should be true when present."
}

Invoke-TestCase "parses PowerShell boolean compatibility value" {
    $cli = ConvertFrom-CliArguments -Arguments @('-DryRun:$false') -OptionSpecs $optionSpecs
    Assert-Equal $false $cli['dry-run'] "Compatibility boolean mismatch."
}

Invoke-TestCase "parses attached canonical boolean value" {
    $cli = ConvertFrom-CliArguments -Arguments @('--dry-run=false') -OptionSpecs $optionSpecs
    Assert-Equal $false $cli['dry-run'] "Attached canonical boolean mismatch."
}

Invoke-TestCase "treats tokens after terminator as positionals" {
    $cli = ConvertFrom-CliArguments -Arguments @('--', '--dry-run') -OptionSpecs $optionSpecs -Positionals @('project')
    Assert-Equal '--dry-run' $cli['project'] "Terminated positional value mismatch."
    Assert-Equal $false $cli['dry-run'] "Option parsing should stop after --."
}

Invoke-TestCase "rejects duplicate canonical option" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--project', 'one', '--project', 'two') -OptionSpecs $optionSpecs
    } '*provided more than once*'
}

Invoke-TestCase "rejects duplicate through aliases" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--project', 'one', '-p', 'two') -OptionSpecs $optionSpecs
    } '*provided more than once*'
}

Invoke-TestCase "rejects duplicate switch" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--dry-run', '-n') -OptionSpecs $optionSpecs
    } '*provided more than once*'
}

Invoke-TestCase "rejects unknown option" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--unknown') -OptionSpecs $optionSpecs
    } 'Unknown option*'
}

Invoke-TestCase "rejects missing value at end" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--project') -OptionSpecs $optionSpecs
    } '*requires a value*'
}

Invoke-TestCase "rejects missing value before another option" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--project', '--dry-run') -OptionSpecs $optionSpecs
    } '*requires a value*'
}

Invoke-TestCase "rejects empty attached value" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--project=') -OptionSpecs $optionSpecs
    } '*requires a value*'
}

Invoke-TestCase "recognizes long help" {
    $cli = ConvertFrom-CliArguments -Arguments @('--help') -OptionSpecs $optionSpecs
    Assert-Equal $true $cli['help'] "--help should be recognized."
}

Invoke-TestCase "recognizes short help" {
    $cli = ConvertFrom-CliArguments -Arguments @('-h') -OptionSpecs $optionSpecs
    Assert-Equal $true $cli['help'] "-h should be recognized."
}

Invoke-TestCase "rejects duplicate help" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--help', '-h') -OptionSpecs $optionSpecs
    } '*provided more than once*'
}

Invoke-TestCase "rejects option and positional duplicate" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @('--project', 'one', 'two') -OptionSpecs $optionSpecs -Positionals @('project')
    } '*both positionally and by option*'
}

Invoke-TestCase "rejects invalid canonical option name" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @() -OptionSpecs @{ ProjectRoot = @{} }
    } '*lowercase kebab-case*'
}

Invoke-TestCase "rejects duplicate alias definitions" {
    Assert-Throws {
        ConvertFrom-CliArguments -Arguments @() -OptionSpecs @{
            first = @{ Aliases = @('-x') }
            second = @{ Aliases = @('-x') }
        }
    } '*configured more than once*'
}

Invoke-TestCase 'preserves spaces and Unicode in separate and attached option values' {
    $path = 'C:\Apollo Sample\媒體.zip'
    $cli = ConvertFrom-CliArguments -Arguments @('--project', $path, "--output=$path") -OptionSpecs $optionSpecs
    Assert-Equal $path $cli['project'] 'Separate path value should be preserved.'
    Assert-Equal $path $cli['output'] 'Attached path value should be preserved.'
}

Invoke-TestCase 'rejects blank CLI values and invalid switch values' {
    Assert-Throws { ConvertFrom-CliArguments -Arguments @('--project', ' ') -OptionSpecs $optionSpecs } '*requires a value*'
    Assert-Throws { ConvertFrom-CliArguments -Arguments @('--output= ') -OptionSpecs $optionSpecs } '*requires a value*'
    Assert-Throws { ConvertFrom-CliArguments -Arguments @('--dry-run=maybe') -OptionSpecs $optionSpecs } '*expects a boolean value*'
}

Invoke-TestCase 'accepts an attached value starting with a dash' {
    $cli = ConvertFrom-CliArguments -Arguments @('--output=-file.zip') -OptionSpecs $optionSpecs
    Assert-Equal '-file.zip' $cli['output'] 'Attached dash-prefixed values should be preserved.'
}

Invoke-TestCase 'parses repeated multi-value options using the reference contract' {
    $cli = ConvertFrom-CliArguments -Arguments @('--items', 'first,second', '--items=third') -OptionSpecs @{ items = @{ Kind = 'MultiValue' } }
    Assert-Equal @('first', 'second', 'third') @($cli['items']) 'Multi-value CLI parsing mismatch.'
}

Invoke-TestCase 'formats binary byte sizes and rejects negative sizes' {
    Assert-Equal '0 B' (Format-CliByteSize -Bytes 0) 'Empty size mismatch.'
    Assert-Equal '1.0 KiB' (Format-CliByteSize -Bytes 1024) 'KiB size mismatch.'
    Assert-Equal '1.0 MiB' (Format-CliByteSize -Bytes 1MB) 'MiB size mismatch.'
    Assert-Throws { Format-CliByteSize -Bytes -1 } '*cannot be negative*'
}

Invoke-TestCase 'NO_COLOR and CI disable transient output while permanent summaries remain visible' {
    $previousNoColor = $env:NO_COLOR
    $previousCI = $env:CI
    try {
        $env:NO_COLOR = '1'
        Assert-False (Test-CliColorEnabled) 'NO_COLOR must disable color.'
        Assert-False (Test-CliProgressEnabled) 'NO_COLOR must disable transient progress.'
        $env:NO_COLOR = $null
        $env:CI = 'true'
        Assert-False (Test-CliColorEnabled) 'CI must disable color.'
        Assert-False (Test-CliProgressEnabled) 'CI must disable transient progress.'
        $output = @(& { Write-CliHeading -Text 'Export'; Write-CliStep -Message 'Packaging'; Write-CliSuccess -Message 'Complete'; Write-CliDetail -Label 'Files' -Value '3'; Write-CliWarning -Message 'Review' } 6>&1) -join "`n"
        foreach ($message in @('Export', 'Packaging', 'Complete', 'Files', '3', 'Review')) {
            Assert-Contains $output $message 'Permanent output should remain readable.'
        }
        Assert-False ($output.Contains([string][char]27)) 'Suppressed output must not contain ANSI escapes.'
    } finally { $env:NO_COLOR = $previousNoColor; $env:CI = $previousCI }
}

Invoke-TestCase 'redirected PowerShell disables color and progress' {
    $pwshPath = (Get-Process -Id $PID).Path
    $commonPath = (Join-Path $projectRoot 'scripts/Cli.Common.ps1').Replace("'", "''")
    $command = ". '$commonPath'; Test-CliColorEnabled; Test-CliProgressEnabled"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $output = (& $pwshPath -NoLogo -NoProfile -EncodedCommand $encoded | Out-String)
    Assert-Equal 0 $LASTEXITCODE 'Redirected CLI child failed.'
    Assert-Equal @('False', 'False') @($output.Trim() -split '\r?\n') 'Redirected host must suppress color and progress.'
}

Complete-TestRun
