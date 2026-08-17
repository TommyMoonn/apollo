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

Complete-TestRun
