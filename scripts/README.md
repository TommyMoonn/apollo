# Apollo Repository Scripts

Apollo repository commands target **PowerShell 7**. `Cli.Common.ps1` is the single argument-parsing owner for public repository scripts.

## CLI conventions

Public commands introduced in later phases must use these conventions:

- `#requires -Version 7.0`
- `Set-StrictMode -Version Latest`
- `$ErrorActionPreference = "Stop"`
- lowercase `--kebab-case` canonical long options
- short aliases where the phase defines them
- PowerShell-style compatibility aliases where an older workflow requires them
- `-h` and `--help` for every public command
- reject duplicate and unknown options
- support `--name=value` for long value options
- support `--` as the positional-option terminator

Individual scripts declare only their accepted option specification and call `ConvertFrom-CliArguments`. They must not implement separate parsing loops.

```powershell
. (Join-Path $PSScriptRoot "Cli.Common.ps1")

$cli = ConvertFrom-CliArguments -Arguments $args -OptionSpecs @{
    project = @{ Aliases = @('-p', '-ProjectRoot') }
    'dry-run' = @{ Kind = 'Switch'; Aliases = @('-n', '-DryRun') }
}

if ($cli['help']) {
    Write-CliHelp @'
Usage: .\scripts\example.ps1 [options]

Options:
  -p, --project <path>    Project root.
  -n, --dry-run           Preview without changes.
  -h, --help              Show this help.
'@
    return
}
```

Compatibility switches may use explicit boolean syntax such as `-DryRun:$false`. Canonical switches normally use presence/absence semantics, although the parser accepts an explicit boolean when attached to a switch.

## Tests

Run all PowerShell tooling tests from the repository root:

```powershell
pwsh -NoProfile -File .\tests\scripts\run.ps1
```
