#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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
        if ($name -notmatch '^[a-z0-9]+(?:-[a-z0-9]+)*$') {
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
                if ([string]::IsNullOrEmpty([string]$attachedValue)) {
                    throw "Option '$optionToken' requires a value."
                }
                $result[$name] = $attachedValue
                continue
            }

            if (($index + 1) -ge $Arguments.Count) {
                throw "Option '$optionToken' requires a value."
            }

            $nextToken = [string]$Arguments[$index + 1]
            if ($nextToken -eq '--' -or @('--help', '-h', '-Help', '-?') -contains $nextToken) {
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

        if (-not $OptionSpecs.ContainsKey($name)) {
            throw "Positional argument '$name' has no matching CLI option specification."
        }
        if (-not $seen.Add($name)) {
            throw "Argument '$name' was provided both positionally and by option."
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
