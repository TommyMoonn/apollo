#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

function Test-SemanticVersion {
    param(
        [Parameter(Mandatory)]
        [string]$Version
    )

    return $Version -cmatch '\A(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-((?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?\z'
}

function Get-CargoManifestPackageInfo {
    param(
        [Parameter(Mandatory)]
        [string]$ManifestPath
    )

    if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
        throw "Cargo manifest does not exist: $ManifestPath"
    }

    $content = [System.IO.File]::ReadAllText($ManifestPath)
    $packageHeader = [regex]::Match($content, '(?m)^[ \t]*\[package\][ \t]*\r?$')
    if (-not $packageHeader.Success) {
        throw "Cargo.toml is missing a [package] section."
    }

    $searchStart = $packageHeader.Index + $packageHeader.Length
    $remaining = $content.Substring($searchStart)
    $nextSection = [regex]::Match($remaining, '(?m)^[ \t]*\[[^\r\n]+\][ \t]*\r?$')
    $sectionEnd = if ($nextSection.Success) {
        $searchStart + $nextSection.Index
    }
    else {
        $content.Length
    }

    $section = $content.Substring($packageHeader.Index, $sectionEnd - $packageHeader.Index)
    $nameMatches = [regex]::Matches(
        $section,
        '(?m)^[ \t]*name[ \t]*=[ \t]*"(?<value>[^"\r\n]+)"[ \t]*(?:#.*)?\r?$'
    )
    $versionMatches = [regex]::Matches(
        $section,
        '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<value>[^"\r\n]+)"[ \t]*(?:#.*)?\r?$'
    )

    if ($nameMatches.Count -ne 1) {
        throw "Cargo.toml [package] must contain exactly one string name."
    }
    if ($versionMatches.Count -ne 1) {
        throw "Cargo.toml [package] must contain exactly one string version."
    }

    $versionGroup = $versionMatches[0].Groups['value']
    return [pscustomobject]@{
        Name = $nameMatches[0].Groups['value'].Value
        Version = $versionGroup.Value
        VersionIndex = $packageHeader.Index + $versionGroup.Index
        VersionLength = $versionGroup.Length
    }
}

function Get-CargoLockRootPackageVersion {
    param(
        [Parameter(Mandatory)]
        [string]$LockPath,

        [Parameter(Mandatory)]
        [string]$PackageName
    )

    if (-not (Test-Path -LiteralPath $LockPath -PathType Leaf)) {
        throw "Cargo lockfile does not exist: $LockPath"
    }

    $content = [System.IO.File]::ReadAllText($LockPath)
    $blocks = [regex]::Matches(
        $content,
        '(?ms)^[ \t]*\[\[package\]\][ \t]*\r?\n(?<body>.*?)(?=^[ \t]*\[\[package\]\]|\z)'
    )
    $rootVersions = [System.Collections.Generic.List[string]]::new()

    foreach ($block in $blocks) {
        $body = $block.Groups['body'].Value
        $nameMatch = [regex]::Match(
            $body,
            '(?m)^[ \t]*name[ \t]*=[ \t]*"(?<value>[^"\r\n]+)"[ \t]*\r?$'
        )
        if (-not $nameMatch.Success -or $nameMatch.Groups['value'].Value -cne $PackageName) {
            continue
        }

        $sourceMatch = [regex]::Match($body, '(?m)^[ \t]*source[ \t]*=')
        if ($sourceMatch.Success) {
            continue
        }

        $versionMatch = [regex]::Match(
            $body,
            '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<value>[^"\r\n]+)"[ \t]*\r?$'
        )
        if (-not $versionMatch.Success) {
            throw "Cargo.lock root package '$PackageName' is missing a version."
        }

        $rootVersions.Add($versionMatch.Groups['value'].Value)
    }

    if ($rootVersions.Count -ne 1) {
        throw "Cargo.lock must contain exactly one root package entry named '$PackageName'."
    }

    return $rootVersions[0]
}

function Get-EffectiveReleaseTag {
    param(
        [AllowNull()]
        $ExplicitTag
    )

    $tagPrefix = 'refs/tags/'
    $isTagRef = $env:GITHUB_REF_TYPE -ceq 'tag' -or (
        -not [string]::IsNullOrWhiteSpace($env:GITHUB_REF) -and
        $env:GITHUB_REF.StartsWith($tagPrefix, [System.StringComparison]::Ordinal)
    )

    $workflowTag = $null
    if ($isTagRef) {
        if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_REF_NAME)) {
            $workflowTag = $env:GITHUB_REF_NAME
        }
        elseif (-not [string]::IsNullOrWhiteSpace($env:GITHUB_REF) -and
            $env:GITHUB_REF.StartsWith($tagPrefix, [System.StringComparison]::Ordinal)) {
            $workflowTag = $env:GITHUB_REF.Substring($tagPrefix.Length)
        }
    }

    if ($null -ne $ExplicitTag) {
        $explicitTagText = [string]$ExplicitTag
        if ([string]::IsNullOrWhiteSpace($explicitTagText)) {
            throw 'Supplied release tag cannot be empty.'
        }
        if ($null -ne $workflowTag -and $explicitTagText -cne $workflowTag) {
            throw "Supplied release tag '$explicitTagText' does not match GitHub tag '$workflowTag'."
        }
        return $explicitTagText
    }

    return $workflowTag
}

function Assert-ReleaseChangelog {
    param(
        [Parameter(Mandatory)]
        [string]$ChangelogPath,

        [Parameter(Mandatory)]
        [string]$Version
    )

    if (-not (Test-Path -LiteralPath $ChangelogPath -PathType Leaf)) {
        throw "CHANGELOG.md does not exist: $ChangelogPath"
    }

    $content = [System.IO.File]::ReadAllText($ChangelogPath)
    $escapedVersion = [regex]::Escape($Version)
    $heading = [regex]::Match(
        $content,
        "(?m)^##[ \t]+\[$escapedVersion\][ \t]+-[ \t]+(?<date>[0-9]{4}-[0-9]{2}-[0-9]{2})[ \t]*\r?$"
    )
    if (-not $heading.Success) {
        throw "CHANGELOG.md is missing a dated [$Version] release heading."
    }

    [datetime]$releaseDate = [datetime]::MinValue
    if (-not [datetime]::TryParseExact(
        $heading.Groups['date'].Value,
        'yyyy-MM-dd',
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::None,
        [ref]$releaseDate
    )) {
        throw "CHANGELOG.md has an invalid release date for [$Version]."
    }

    $link = [regex]::Match(
        $content,
        "(?m)^\[$escapedVersion\]:[ \t]+(?<url>\S+)[ \t]*\r?$"
    )
    if (-not $link.Success) {
        throw "CHANGELOG.md is missing the [$Version] comparison link."
    }

    $expectedTag = "v$Version"
    if (-not $link.Groups['url'].Value.Contains($expectedTag, [System.StringComparison]::Ordinal)) {
        throw "CHANGELOG.md [$Version] comparison link does not reference '$expectedTag'."
    }
}

function Invoke-ReleaseValidation {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [AllowNull()]
        $Tag,

        [switch]$ReleaseChangelog
    )

    $resolvedRoot = Resolve-RepositoryRoot -Path $ProjectRoot
    $manifestPath = Join-Path $resolvedRoot 'Cargo.toml'
    $lockPath = Join-Path $resolvedRoot 'Cargo.lock'
    $manifest = Get-CargoManifestPackageInfo -ManifestPath $manifestPath

    if ($manifest.Name -cne 'apollo') {
        throw "Cargo.toml package name must be 'apollo', found '$($manifest.Name)'."
    }
    if (-not (Test-SemanticVersion -Version $manifest.Version)) {
        throw "Cargo.toml package version '$($manifest.Version)' is not a valid semantic version."
    }

    $lockVersion = Get-CargoLockRootPackageVersion -LockPath $lockPath -PackageName 'apollo'
    if ($lockVersion -cne $manifest.Version) {
        throw "Cargo.lock root package version '$lockVersion' does not match Cargo.toml version '$($manifest.Version)'."
    }

    $effectiveTag = Get-EffectiveReleaseTag -ExplicitTag $Tag
    if ($null -ne $effectiveTag) {
        $expectedTag = "v$($manifest.Version)"
        if ($effectiveTag -cne $expectedTag) {
            throw "Release tag '$effectiveTag' does not match package version '$($manifest.Version)'; expected '$expectedTag'."
        }
    }

    if ($ReleaseChangelog) {
        Assert-ReleaseChangelog -ChangelogPath (Join-Path $resolvedRoot 'CHANGELOG.md') -Version $manifest.Version
    }

    return [pscustomobject]@{
        PackageName = $manifest.Name
        Version = $manifest.Version
        Tag = $effectiveTag
    }
}

function Invoke-CheckReleaseCli {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Arguments
    )

    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
        tag = @{ Aliases = @('-t', '-Tag') }
        'release-changelog' = @{ Kind = 'Switch'; Aliases = @('-ReleaseChangelog') }
    }

    if ($cli['help']) {
        Write-CliHelp @'
Usage: .\scripts\check-release.ps1 [options]

Options:
  -p, --project <path>          Project root to validate.
  -t, --tag <tag>               Require an exact v<version> release tag.
  --release-changelog           Require a dated release heading and matching link.
  -h, --help                    Show this help.

When GITHUB_REF_TYPE=tag, GITHUB_REF_NAME is used as the tag if --tag is omitted.
'@
        return
    }

    $validationArgs = @{
        ProjectRoot = [string]$cli['project']
        Tag = $cli['tag']
        ReleaseChangelog = [bool]$cli['release-changelog']
    }
    $result = Invoke-ReleaseValidation @validationArgs

    Write-Host "Release metadata valid for $($result.PackageName) $($result.Version)."
}

if ($MyInvocation.InvocationName -ne '.') {
    Invoke-CheckReleaseCli -Arguments $args
}
