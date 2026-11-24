#requires -Version 7.0
# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
# Modified for Apollo on 2026-11-24.

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

function Add-CliMultiValue {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.List[object]]$Target,

        [Parameter(Mandatory)]
        [object]$Value
    )

    if ($Value -is [string]) {
        foreach ($item in ([string]$Value -split ',')) {
            $trimmed = $item.Trim()
            if (-not [string]::IsNullOrWhiteSpace($trimmed)) {
                $Target.Add($trimmed)
            }
        }
        return
    }

    if ($Value -isnot [System.Collections.IEnumerable]) {
        $Target.Add($Value)
        return
    }

    foreach ($item in $Value) {
        Add-CliMultiValue -Target $Target -Value $item
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
        if ($name -cnotmatch '^[a-z0-9]+(?:-[a-z0-9]+)*$') {
            throw "Canonical CLI option name '$name' must use lowercase kebab-case."
        }

        $spec = $OptionSpecs[$name]
        if ($spec -isnot [hashtable]) {
            throw "CLI option '$name' must be configured with a hashtable specification."
        }

        $kind = if ($spec.ContainsKey('Kind')) { [string]$spec['Kind'] } else { 'Value' }
        if ($kind -notin @('Switch', 'Value', 'MultiValue')) {
            throw "CLI option '$name' has unsupported kind '$kind'."
        }

        if ($spec.ContainsKey('Default')) {
            $result[$name] = $spec['Default']
        }
        elseif ($kind -eq 'Switch') {
            $result[$name] = $false
        }
        elseif ($kind -eq 'MultiValue') {
            $result[$name] = @()
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

            $firstOccurrence = $seen.Add($name)
            if (-not $firstOccurrence -and $kind -ne 'MultiValue') {
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

            if (-not $hasAttachedValue) {
                if (($index + 1) -ge $Arguments.Count) { throw "Option '$optionToken' requires a value." }
                $nextToken = [string]$Arguments[$index + 1]
                if ($nextToken.StartsWith('-', [System.StringComparison]::Ordinal)) {
                    throw "Option '$optionToken' requires a value. Use the attached form, such as --name=-value, when a value must begin with '-'."
                }
                $index++
                $attachedValue = $Arguments[$index]
            }
            if ([string]::IsNullOrWhiteSpace([string]$attachedValue)) { throw "Option '$optionToken' requires a value." }
            if ($kind -eq 'MultiValue') {
                $items = [System.Collections.Generic.List[object]]::new()
                if (-not $firstOccurrence) {
                    foreach ($existing in @($result[$name])) { $items.Add($existing) }
                }
                Add-CliMultiValue -Target $items -Value $attachedValue
                if ($items.Count -eq 0) { throw "Option '$optionToken' requires a value." }
                $result[$name] = $items.ToArray()
            }
            else { $result[$name] = $attachedValue }
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

if (-not (Get-Variable -Name CliActiveProgressIds -Scope Script -ErrorAction SilentlyContinue)) {
    $script:CliActiveProgressIds = [System.Collections.Generic.HashSet[int]]::new()
}

function Test-CliColorEnabled {
    if ($null -ne $env:NO_COLOR -or -not [string]::IsNullOrWhiteSpace($env:CI)) {
        return $false
    }

    try {
        if ([Console]::IsOutputRedirected) {
            return $false
        }

        $null = $Host.UI.RawUI.ForegroundColor
        return $true
    } catch {
        return $false
    }
}

function Test-CliProgressEnabled {
    if ($null -ne $env:NO_COLOR -or -not [string]::IsNullOrWhiteSpace($env:CI)) {
        return $false
    }

    try {
        if ([Console]::IsOutputRedirected) {
            return $false
        }

        $null = $Host.UI.RawUI
        return $true
    } catch {
        return $false
    }
}

function Get-CliToneColor {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('Normal', 'Success', 'Warning', 'Important', 'Muted')]
        [string]$Tone
    )

    switch ($Tone) {
        'Success' { return [System.ConsoleColor]::Green }
        'Warning' { return [System.ConsoleColor]::Yellow }
        'Important' { return [System.ConsoleColor]::Cyan }
        'Muted' { return [System.ConsoleColor]::DarkGray }
        default { return $null }
    }
}

function Write-CliText {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Text,

        [ValidateSet('Normal', 'Success', 'Warning', 'Important', 'Muted')]
        [string]$Tone = 'Normal',

        [switch]$NoNewline
    )

    $color = if (Test-CliColorEnabled) { Get-CliToneColor -Tone $Tone } else { $null }
    if ($null -ne $color) {
        if ($NoNewline) {
            Write-Host $Text -ForegroundColor $color -NoNewline
        } else {
            Write-Host $Text -ForegroundColor $color
        }
        return
    }

    if ($NoNewline) {
        Write-Host $Text -NoNewline
    } else {
        Write-Host $Text
    }
}

function Format-CliByteSize {
    param(
        [Parameter(Mandatory)]
        [long]$Bytes
    )

    if ($Bytes -lt 0) {
        throw 'Byte size cannot be negative.'
    }

    $culture = [System.Globalization.CultureInfo]::InvariantCulture
    if ($Bytes -ge 1GB) {
        return [string]::Format($culture, '{0:0.00} GiB', ($Bytes / 1GB))
    }
    if ($Bytes -ge 1MB) {
        return [string]::Format($culture, '{0:0.0} MiB', ($Bytes / 1MB))
    }
    if ($Bytes -ge 1KB) {
        return [string]::Format($culture, '{0:0.0} KiB', ($Bytes / 1KB))
    }

    return "$Bytes B"
}

function Write-CliHeading {
    param(
        [Parameter(Mandatory)]
        [string]$Text
    )

    Write-CliText -Text $Text
}

function Write-CliDetail {
    param(
        [Parameter(Mandatory)]
        [string]$Label,

        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Value,

        [ValidateSet('Normal', 'Success', 'Warning', 'Important', 'Muted')]
        [string]$ValueTone = 'Normal'
    )

    Write-CliText -Text ("  {0,-12} " -f $Label) -Tone 'Muted' -NoNewline
    Write-CliText -Text $Value -Tone $ValueTone
}

function Write-CliStatus {
    param(
        [Parameter(Mandatory)]
        [string]$Label,

        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Message,

        [ValidateSet('Normal', 'Success', 'Warning', 'Important', 'Muted')]
        [string]$Tone = 'Normal',

        [ValidateSet('Normal', 'Success', 'Warning', 'Important', 'Muted')]
        [string]$MessageTone = 'Normal'
    )

    Write-CliText -Text ("  {0,-12} " -f $Label) -Tone $Tone -NoNewline
    Write-CliText -Text $Message -Tone $MessageTone
}

function Write-CliSuccess {
    param(
        [Parameter(Mandatory)]
        [string]$Message
    )

    Write-CliText -Text "✓ $Message" -Tone 'Success'
}

function Write-CliWarning {
    param(
        [Parameter(Mandatory)]
        [string]$Message
    )

    Write-CliText -Text "! $Message" -Tone 'Warning'
}

function Write-CliStep {
    param(
        [Parameter(Mandatory)]
        [string]$Message,

        [int]$Current,

        [int]$Total
    )

    $hasCurrent = $PSBoundParameters.ContainsKey('Current')
    $hasTotal = $PSBoundParameters.ContainsKey('Total')
    if ($hasCurrent -ne $hasTotal) {
        throw 'Write-CliStep requires both -Current and -Total when numbering steps.'
    }

    if ($hasCurrent) {
        if ($Current -lt 1 -or $Total -lt 1 -or $Current -gt $Total) {
            throw 'Write-CliStep requires 1 <= Current <= Total.'
        }
        Write-CliText -Text "[$Current/$Total] $Message"
        return
    }

    Write-CliText -Text $Message
}

function Write-CliProgress {
    param(
        [Parameter(Mandatory)]
        [string]$Activity,

        [Parameter(Mandatory)]
        [string]$Status,

        [Parameter(Mandatory)]
        [ValidateRange(0, 2147483647)]
        [int]$Current,

        [Parameter(Mandatory)]
        [ValidateRange(1, 2147483647)]
        [int]$Total,

        [ValidateRange(0, 2147483647)]
        [int]$Id = 1
    )

    if ($Current -gt $Total -or -not (Test-CliProgressEnabled)) {
        return
    }

    $percentComplete = [Math]::Min(100, [Math]::Floor(100 * ($Current / [double]$Total)))
    try {
        Write-Progress `
            -Id $Id `
            -Activity $Activity `
            -Status "$Status $Current / $Total" `
            -PercentComplete $percentComplete
        [void]$script:CliActiveProgressIds.Add($Id)
    } catch {
        # Progress is presentation-only. Unsupported host rendering must not fail the command.
    }
}

function Complete-CliProgress {
    param(
        [ValidateRange(0, 2147483647)]
        [int]$Id = 1,

        [string]$Activity = 'Working'
    )

    if (-not $script:CliActiveProgressIds.Contains($Id)) {
        return
    }

    try {
        Write-Progress -Id $Id -Activity $Activity -Completed
    } catch {
        # Completion is safe from finally paths even when the host cannot render progress.
    } finally {
        [void]$script:CliActiveProgressIds.Remove($Id)
    }
}
