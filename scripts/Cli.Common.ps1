#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Preserve Unicode filenames in redirected CLI output.
if ([Console]::IsOutputRedirected) {
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
}

function ConvertTo-CliBoolean {
    param(
        [Parameter(Mandatory)]
        [object]$Value,

        [Parameter(Mandatory)]
        [string]$OptionName
    )

    if ($Value -is [bool]) {
        return $Value
    }

    $text = ([string]$Value).Trim().ToLowerInvariant()
    switch ($text) {
        'true' { return $true }
        '$true' { return $true }
        '1' { return $true }
        'yes' { return $true }
        'on' { return $true }
        'false' { return $false }
        '$false' { return $false }
        '0' { return $false }
        'no' { return $false }
        'off' { return $false }
        default { throw "Option '$OptionName' expects a boolean value." }
    }
}

function ConvertTo-CliInt {
    param(
        [Parameter(Mandatory)]
        [object]$Value,

        [Parameter(Mandatory)]
        [string]$OptionName,

        [int]$Minimum = [int]::MinValue,

        [int]$Maximum = [int]::MaxValue
    )

    if ($Minimum -gt $Maximum) {
        throw "Minimum cannot be greater than maximum for option '$OptionName'."
    }

    $text = ([string]$Value).Trim()
    [int]$parsed = 0
    $parsedSuccessfully = [int]::TryParse(
        $text,
        [System.Globalization.NumberStyles]::Integer,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [ref]$parsed
    )

    if (-not $parsedSuccessfully) {
        throw "Option '$OptionName' expects an integer value."
    }

    if ($parsed -lt $Minimum -or $parsed -gt $Maximum) {
        throw "Option '$OptionName' must be between $Minimum and $Maximum."
    }

    return $parsed
}

function Resolve-CliChoice {
    param(
        [Parameter(Mandatory)]
        [object]$Value,

        [Parameter(Mandatory)]
        [string]$OptionName,

        [Parameter(Mandatory)]
        [ValidateNotNullOrEmpty()]
        [string[]]$AllowedValues
    )

    $text = ([string]$Value).Trim()
    foreach ($allowedValue in $AllowedValues) {
        if ($text.Equals($allowedValue, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $allowedValue
        }
    }

    $displayValues = $AllowedValues -join ', '
    throw "Option '$OptionName' expects one of: $displayValues."
}

function ConvertFrom-CliArguments {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Arguments,

        [Parameter(Mandatory)]
        [hashtable]$OptionSpecs,

        [string[]]$Positionals = @()
    )

    if ($OptionSpecs.ContainsKey('help')) {
        throw "The CLI option name 'help' is reserved."
    }

    $result = [ordered]@{ help = $false }
    $aliases = @{}
    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)

    foreach ($nameObject in $OptionSpecs.Keys) {
        $name = [string]$nameObject
        if ($name -cnotmatch '^[a-z0-9]+(?:-[a-z0-9]+)*$') {
            throw "Canonical CLI option name '$name' must use lowercase kebab-case."
        }

        $spec = $OptionSpecs[$name]
        if ($spec -isnot [hashtable]) {
            throw "CLI option '$name' must be configured with a hashtable specification."
        }

        $kind = if ($spec.ContainsKey('Kind')) { [string]$spec['Kind'] } else { 'Value' }
        if ($kind -notin @('Switch', 'Value')) {
            throw "CLI option '$name' has unsupported kind '$kind'."
        }

        if ($spec.ContainsKey('Default')) {
            $result[$name] = $spec['Default']
        }
        elseif ($kind -eq 'Switch') {
            $result[$name] = $false
        }
        else {
            $result[$name] = $null
        }

        $optionAliases = [System.Collections.Generic.List[string]]::new()
        if (-not $spec.ContainsKey('Canonical') -or [bool]$spec['Canonical']) {
            $optionAliases.Add("--$name")
        }

        if ($spec.ContainsKey('Aliases')) {
            foreach ($aliasObject in @($spec['Aliases'])) {
                $alias = [string]$aliasObject
                if ([string]::IsNullOrWhiteSpace($alias)) {
                    throw "CLI option '$name' contains an empty alias."
                }
                if (-not $alias.StartsWith('-', [System.StringComparison]::Ordinal)) {
                    throw "CLI alias '$alias' for '$name' must start with '-'."
                }
                if ($alias.Contains('=') -or $alias.Contains(':')) {
                    throw "CLI alias '$alias' for '$name' cannot contain '=' or ':'."
                }
                $optionAliases.Add($alias)
            }
        }

        foreach ($alias in $optionAliases) {
            $normalizedAlias = $alias.ToLowerInvariant()
            if ($normalizedAlias -in @('--help', '-h', '-help', '-?')) {
                throw "CLI alias '$alias' conflicts with a reserved help option."
            }
            if ($aliases.ContainsKey($normalizedAlias)) {
                throw "CLI alias '$alias' is configured more than once."
            }
            $aliases[$normalizedAlias] = $name
        }
    }

    foreach ($positional in $Positionals) {
        if (-not $OptionSpecs.ContainsKey($positional)) {
            throw "Positional argument '$positional' has no matching CLI option specification."
        }
    }

    $positionIndex = 0
    $parseOptions = $true

    for ($index = 0; $index -lt $Arguments.Count; $index++) {
        $argumentObject = $Arguments[$index]
        $token = [string]$argumentObject

        if ($parseOptions -and $token -eq '--') {
            $parseOptions = $false
            continue
        }

        if ($parseOptions -and @('--help', '-h', '-Help', '-?') -contains $token) {
            if (-not $seen.Add('help')) {
                throw "Option '$token' was provided more than once."
            }
            $result['help'] = $true
            continue
        }

        if ($parseOptions -and $token.StartsWith('-', [System.StringComparison]::Ordinal)) {
            $optionToken = $token
            $attachedValue = $null
            $hasAttachedValue = $false

            $separatorIndex = $token.IndexOf('=')
            if ($separatorIndex -gt 0) {
                $optionToken = $token.Substring(0, $separatorIndex)
                $attachedValue = $token.Substring($separatorIndex + 1)
                $hasAttachedValue = $true
            }
            elseif (-not $token.StartsWith('--', [System.StringComparison]::Ordinal)) {
                $separatorIndex = $token.IndexOf(':')
                if ($separatorIndex -gt 0) {
                    $optionToken = $token.Substring(0, $separatorIndex)
                    $attachedValue = $token.Substring($separatorIndex + 1)
                    $hasAttachedValue = $true
                }
            }

            $normalizedOption = $optionToken.ToLowerInvariant()
            if (-not $aliases.ContainsKey($normalizedOption)) {
                throw "Unknown option '$optionToken'. Run with --help to see supported options."
            }

            $name = [string]$aliases[$normalizedOption]
            $spec = $OptionSpecs[$name]
            $kind = if ($spec.ContainsKey('Kind')) { [string]$spec['Kind'] } else { 'Value' }

            if (-not $seen.Add($name)) {
                throw "Option '$optionToken' was provided more than once."
            }

            if ($kind -eq 'Switch') {
                $result[$name] = if ($hasAttachedValue) {
                    ConvertTo-CliBoolean -Value $attachedValue -OptionName $optionToken
                }
                else {
                    $true
                }
                continue
            }

            if ($hasAttachedValue) {
                if ([string]::IsNullOrWhiteSpace([string]$attachedValue)) {
                    throw "Option '$optionToken' requires a value."
                }
                $result[$name] = $attachedValue
                continue
            }

            if (($index + 1) -ge $Arguments.Count) {
                throw "Option '$optionToken' requires a value."
            }

            $nextToken = [string]$Arguments[$index + 1]
            if ([string]::IsNullOrWhiteSpace($nextToken) -or $nextToken -eq '--' -or @('--help', '-h', '-Help', '-?') -contains $nextToken) {
                throw "Option '$optionToken' requires a value."
            }

            if ($nextToken.StartsWith('-', [System.StringComparison]::Ordinal)) {
                throw "Option '$optionToken' requires a value. Use the attached form, such as --name=-value, when a value must begin with '-'."
            }

            $index++
            $result[$name] = $Arguments[$index]
            continue
        }

        if ($positionIndex -ge $Positionals.Count) {
            throw "Unexpected positional argument '$token'. Run with --help to see usage."
        }

        $name = $Positionals[$positionIndex]
        $positionIndex++

        if (-not $seen.Add($name)) {
            throw "Argument '$name' was provided both positionally and by option."
        }

        if ([string]::IsNullOrWhiteSpace($token)) {
            throw "Argument '$name' requires a value."
        }
        $result[$name] = $argumentObject
    }

    return ,$result
}

function Write-CliHelp {
    param(
        [Parameter(Mandatory)]
        [string]$Text
    )

    Write-Host $Text.Trim()
}

# Progress is presentation only; redirected and CI output stays readable.
function Test-CliProgressEnabled {
    if ($null -ne $env:NO_COLOR -or -not [string]::IsNullOrWhiteSpace($env:CI)) { return $false }
    try { return (-not [Console]::IsOutputRedirected -and $null -ne $Host.UI.RawUI) }
    catch { return $false }
}

function Write-CliProgress {
    param(
        [Parameter(Mandatory)][string]$Activity,
        [Parameter(Mandatory)][string]$Status,
        [ValidateRange(0, 2147483647)][int]$Current,
        [ValidateRange(1, 2147483647)][int]$Total,
        [ValidateRange(0, 2147483647)][int]$Id = 1
    )

    if (-not (Test-CliProgressEnabled)) { return }
    $percent = -1
    if ($PSBoundParameters.ContainsKey('Total')) {
        $percent = [int][Math]::Min(100, [Math]::Floor(100 * $Current / [double]$Total))
    }
    try { Write-Progress -Id $Id -Activity $Activity -Status $Status -PercentComplete $percent }
    catch { # An unsupported host must not interrupt filesystem operations.
    }
}

function Complete-CliProgress {
    param([int]$Id = 1, [string]$Activity = 'Working')

    if (-not (Test-CliProgressEnabled)) { return }
    try { Write-Progress -Id $Id -Activity $Activity -Completed }
    catch { # Completion is safe in finally paths.
    }
}
