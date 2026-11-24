#requires -Version 7.0

# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'Cli.Common.ps1')
. (Join-Path $PSScriptRoot 'Repository.Common.ps1')

function Assert-StableReleaseVersion {
    param([Parameter(Mandatory)][string]$Version)

    if ($Version -cnotmatch '\A(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z') {
        throw "Version '$Version' must be a stable X.Y.Z version."
    }
    foreach ($part in $Version.Split('.')) {
        # VERSIONINFO stores each numeric component in a 16-bit unsigned field.
        [uint16]$number = 0
        if (-not [uint16]::TryParse($part, [ref]$number)) {
            throw "Version '$Version' exceeds the Windows resource component limit (65535)."
        }
    }
}

function Get-CargoManifestPackageInfo {
    param(
        [Parameter(Mandatory)]
        [string]$ManifestPath
    )

    if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
        throw "Cargo manifest does not exist: $ManifestPath"
    }

    return ConvertFrom-CargoManifestPackageInfo -Content ([System.IO.File]::ReadAllText($ManifestPath))
}

function ConvertFrom-CargoManifestPackageInfo {
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Content)

    $packageHeaders = [regex]::Matches($content, '(?m)^[ \t]*\[package\][ \t]*(?:#[^\r\n]*)?\r?$')
    if ($packageHeaders.Count -ne 1) {
        throw 'Cargo.toml must contain exactly one [package] section.'
    }
    $packageHeader = $packageHeaders[0]

    $searchStart = $packageHeader.Index + $packageHeader.Length
    $remaining = $content.Substring($searchStart)
    $nextSection = [regex]::Match($remaining, '(?m)^[ \t]*\[[^\r\n]+\][ \t]*(?:#[^\r\n]*)?\r?$')
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

    if ($nameMatches.Count -ne 1 -or [regex]::Matches($section, '(?m)^[ \t]*name[ \t]*=').Count -ne 1) {
        throw "Cargo.toml [package] must contain exactly one string name."
    }
    if ($versionMatches.Count -ne 1 -or [regex]::Matches($section, '(?m)^[ \t]*version[ \t]*=').Count -ne 1) {
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

function Get-CargoLockRootPackageInfo {
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
    $rootPackages = [System.Collections.Generic.List[object]]::new()

    foreach ($block in $blocks) {
        $body = $block.Groups['body'].Value
        $nameMatches = [regex]::Matches(
            $body,
            '(?m)^[ \t]*name[ \t]*=[ \t]*"(?<value>[^"\r\n]+)"[ \t]*\r?$'
        )
        if (-not @($nameMatches | Where-Object { $_.Groups['value'].Value -ceq $PackageName }).Count) {
            continue
        }

        $sourceMatch = [regex]::Match($body, '(?m)^[ \t]*source[ \t]*=')
        if ($sourceMatch.Success) {
            continue
        }

        if ($nameMatches.Count -ne 1 -or [regex]::Matches($body, '(?m)^[ \t]*name[ \t]*=').Count -ne 1) {
            throw "Cargo.lock package '$PackageName' has duplicate names."
        }
        $versionMatches = [regex]::Matches(
            $body,
            '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<value>[^"\r\n]+)"[ \t]*\r?$'
        )
        if ($versionMatches.Count -ne 1 -or [regex]::Matches($body, '(?m)^[ \t]*version[ \t]*=').Count -ne 1) {
            throw "Cargo.lock root package '$PackageName' must contain exactly one version."
        }
        $versionGroup = $versionMatches[0].Groups['value']
        $rootPackages.Add([pscustomobject]@{
            Version = $versionGroup.Value
            VersionIndex = $block.Groups['body'].Index + $versionGroup.Index
            VersionLength = $versionGroup.Length
        })
    }

    if ($rootPackages.Count -ne 1) {
        throw "Cargo.lock must contain exactly one root package entry named '$PackageName'."
    }

    return $rootPackages[0]
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
        $refTag = if ($env:GITHUB_REF -and $env:GITHUB_REF.StartsWith($tagPrefix, [System.StringComparison]::Ordinal)) {
            $env:GITHUB_REF.Substring($tagPrefix.Length)
        }
        $workflowTag = if ($env:GITHUB_REF_NAME) { $env:GITHUB_REF_NAME } else { $refTag }
        if ([string]::IsNullOrWhiteSpace($workflowTag)) {
            throw 'GitHub tag context is missing the tag name.'
        }
        if ($refTag -and $refTag -cne $workflowTag) {
            throw 'GitHub tag name and ref disagree.'
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

function Get-ReleaseChangelogDate {
    param(
        [Parameter(Mandatory)][string]$ChangelogPath,
        [Parameter(Mandatory)][string]$Version
    )

    $content = [System.IO.File]::ReadAllText($ChangelogPath)
    $escapedVersion = [regex]::Escape($Version)
    $headings = [regex]::Matches($content, "(?m)^##[ \t]+\[$escapedVersion\][^\r\n]*")
    if ($headings.Count -ne 1) {
        throw "CHANGELOG.md must contain exactly one [$Version] release heading."
    }
    $heading = [regex]::Match($headings[0].Value, "\A##[ \t]+\[$escapedVersion\][ \t]+-[ \t]+(?<date>[0-9]{4}-[0-9]{2}-[0-9]{2})[ \t]*\z")
    [datetime]$releaseDate = [datetime]::MinValue
    if (-not $heading.Success -or -not [datetime]::TryParseExact(
        $heading.Groups['date'].Value, 'yyyy-MM-dd',
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::None, [ref]$releaseDate
    )) {
        throw "CHANGELOG.md requires a valid yyyy-MM-dd release date for [$Version]."
    }

    $links = [regex]::Matches($content, "(?m)^[ \t]*\[$escapedVersion\]:[^\r\n]*")
    if ($links.Count -ne 1) {
        throw "CHANGELOG.md must contain exactly one [$Version] comparison link."
    }
    $comparison = [regex]::Match($links[0].Value, "\A[ \t]*\[$escapedVersion\]:[ \t]+https://github\.com/TommyMoonn/apollo/compare/v(?<previous>[0-9]+\.[0-9]+\.[0-9]+)\.\.\.v$escapedVersion[ \t]*\z")
    if (-not $comparison.Success) {
        throw "CHANGELOG.md [$Version] comparison link must target exactly v$Version in Apollo."
    }
    Assert-StableReleaseVersion -Version $comparison.Groups['previous'].Value
    return $heading.Groups['date'].Value
}

function Read-ReleaseNote {
    param(
        [Parameter(Mandatory)][string]$ProjectRoot,
        [Parameter(Mandatory)][string]$Version
    )

    Assert-StableReleaseVersion -Version $Version
    $date = Get-ReleaseChangelogDate -ChangelogPath (Join-Path $ProjectRoot 'CHANGELOG.md') -Version $Version
    $path = Join-Path $ProjectRoot "release-notes/v$Version.md"
    $body = [System.Text.UTF8Encoding]::new($false, $true).GetString([System.IO.File]::ReadAllBytes($path))
    if ($body -match '\{\{[^}]+\}\}|\bv?X\.[XY]\.[XZ]\b') {
        throw "Release note v$Version contains an unfilled placeholder."
    }
    # The body is published verbatim; section extraction is only for validation.
    $headings = [regex]::Matches($body, '(?m)^#{1,2}[ \t]+[^\r\n]*')
    if ($headings.Count -ne 2 -or $headings[0].Value.TrimEnd() -cne '## Downloads' -or
        $headings[1].Value.TrimEnd() -cne '## Changes' -or $headings[0].Index -ne 0) {
        throw "Release note v$Version must contain Downloads followed by Changes, without an extra header."
    }
    $downloadsStart = $headings[0].Index + $headings[0].Length
    $downloads = $body.Substring($downloadsStart, $headings[1].Index - $downloadsStart)
    $lines = @($downloads -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $assetUrls = @('Apollo-Setup-x64.exe', 'Apollo-windows-x64.zip') | ForEach-Object {
        "https://github.com/TommyMoonn/apollo/releases/download/v$Version/$_"
    }
    $expected = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($url in $assetUrls) { [void]$expected.Add($url) }
    foreach ($line in $lines) {
        $link = [regex]::Match($line, '\A[-*][ \t]+\[[^\]\r\n]+\]\((?<url>[^\s()]+)\)[ \t]*\z')
        if (-not $link.Success -or -not $expected.Remove($link.Groups['url'].Value)) {
            throw "Release note v$Version contains an invalid or duplicate download link."
        }
    }
    if ($expected.Count -ne 0) {
        throw "Release note v$Version must link to both version-specific Windows assets."
    }
    $changes = $body.Substring($headings[1].Index + $headings[1].Length)
    if ($changes -cnotmatch '(?m)^[-*][ \t]+\S[^\r\n]*') {
        throw "Release note v$Version must contain at least one user-facing change."
    }
    # Also reject conflicting Apollo release download URLs elsewhere in the body.
    foreach ($match in [regex]::Matches($changes, 'https://github\.com/TommyMoonn/apollo/releases/download/[^\s)>]+')) {
        if ($match.Value -cnotin $assetUrls) {
            throw "Release note v$Version contains a mismatched download destination."
        }
    }
    return [pscustomobject]@{ Version = $Version; Date = $Date; Path = $path; Body = $body }
}

function Invoke-ReleaseValidation {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [AllowNull()]
        $Tag,

        [switch]$ReleaseChangelog,
        [switch]$RequireChangelog
    )

    $resolvedRoot = Resolve-RepositoryRoot -Path $ProjectRoot
    $manifestPath = Join-Path $resolvedRoot 'Cargo.toml'
    $lockPath = Join-Path $resolvedRoot 'Cargo.lock'
    $manifest = Get-CargoManifestPackageInfo -ManifestPath $manifestPath

    if ($manifest.Name -cne 'apollo') {
        throw "Cargo.toml package name must be 'apollo', found '$($manifest.Name)'."
    }
    Assert-StableReleaseVersion -Version $manifest.Version

    $lockVersion = (Get-CargoLockRootPackageInfo -LockPath $lockPath -PackageName 'apollo').Version
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

    $note = $null
    if ($RequireChangelog -or $ReleaseChangelog) {
        if ($RequireChangelog -and $null -eq $effectiveTag) {
            throw '--require-changelog requires an explicit or inherited release tag.'
        }
        if ($RequireChangelog) {
            $note = Read-ReleaseNote -ProjectRoot $resolvedRoot -Version $manifest.Version
        }
        else {
            $null = Get-ReleaseChangelogDate -ChangelogPath (Join-Path $resolvedRoot 'CHANGELOG.md') -Version $manifest.Version
        }
    }

    return [pscustomobject]@{
        PackageName = $manifest.Name
        Version = $manifest.Version
        Tag = $effectiveTag
        Note = $note
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
        'require-changelog' = @{ Kind = 'Switch'; Aliases = @('-RequireChangelogEntry') }
    }

    if ($cli['help']) {
        Write-CliHelp @'
Usage: .\scripts\check-release.ps1 [options]

Options:
  -p, --project <path>          Project root to validate.
  -t, --tag <tag>               Require an exact v<version> release tag.
  --require-changelog           Require an exact tag, changelog and canonical release note.
  --release-changelog           Legacy: check only the dated heading and comparison link.
  -h, --help                    Show this help.

When GITHUB_REF_TYPE=tag, GITHUB_REF_NAME is used as the tag if --tag is omitted.
'@
        return
    }

    $validationArgs = @{
        ProjectRoot = [string]$cli['project']
        Tag = $cli['tag']
        ReleaseChangelog = [bool]$cli['release-changelog']
        RequireChangelog = [bool]$cli['require-changelog']
    }
    $result = Invoke-ReleaseValidation @validationArgs

    Write-CliHeading 'Release validation'
    Write-CliSuccess "Release metadata valid for $($result.PackageName) $($result.Version)."
    if ($result.Note) { Write-CliDetail -Label 'Release note' -Value $result.Note.Path }
}

if ($MyInvocation.InvocationName -ne '.') {
    Invoke-CheckReleaseCli -Arguments $args
}
