#requires -Version 7.0

# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'check-release.ps1')

function Assert-ReleaseCandidateArguments([string]$Commit, [string]$Repository) {
    if ($Commit -cnotmatch '\A[0-9a-f]{40}\z') { throw 'Expected a full lowercase GitHub commit SHA.' }
    if ($Repository -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9-]{0,38}/[A-Za-z0-9_.-]{1,100}\z' -or
        $Repository.Split('/')[1] -in @('.', '..')) { throw 'Expected a GitHub owner/repository name.' }
}

function Read-CandidateGit([string]$ProjectRoot, [string[]]$Arguments) {
    $result = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments (@('--no-optional-locks', '--no-replace-objects') + $Arguments)
    return $result.StdOut
}

function Assert-CandidateCheckout([string]$ProjectRoot, [string]$Commit, [string]$Repository) {
    Assert-ReleaseCandidateArguments $Commit $Repository
    $top = (Read-CandidateGit $ProjectRoot @('rev-parse', '--show-toplevel')).Trim()
    if (-not [IO.Path]::GetFullPath($top).Equals($ProjectRoot, (Get-RepositoryPathComparison))) {
        throw 'Project must be the root of the candidate checkout.'
    }
    $head = (Read-CandidateGit $ProjectRoot @('rev-parse', '--verify', 'HEAD')).Trim()
    if ($head -cne $Commit) { throw "Checkout HEAD does not match candidate $Commit." }
    if ((Read-CandidateGit $ProjectRoot @('rev-parse', '--is-shallow-repository')).Trim() -cne 'false') {
        throw 'Candidate detection requires complete Git history.'
    }
    if ((Read-CandidateGit $ProjectRoot @('status', '--porcelain=v1', '--untracked-files=normal')).Length -ne 0) {
        throw 'Candidate detection requires a clean checkout.'
    }
    $origin = (Read-CandidateGit $ProjectRoot @('remote', 'get-url', 'origin')).Trim()
    $remote = [regex]::Match($origin, '\A(?:https://github\.com/|git@github\.com:|ssh://git@github\.com/)(?<repo>[^/\s]+/[^/\s]+?)(?:\.git)?\z', [Text.RegularExpressions.RegexOptions]::IgnoreCase)
    if (-not $remote.Success -or $remote.Groups['repo'].Value -ine $Repository) {
        throw 'Git origin does not match the supplied GitHub repository.'
    }
    if ($env:GITHUB_REPOSITORY -and $env:GITHUB_REPOSITORY -ine $Repository) {
        throw 'GitHub repository context does not match the supplied repository.'
    }
}

function Assert-CandidateTrackedFiles([string]$ProjectRoot, [string]$Commit, [string[]]$Paths, [switch]$CheckWorktree) {
    foreach ($path in $Paths) {
        $entry = (Read-CandidateGit $ProjectRoot @('ls-tree', $Commit, '--', $path)).TrimEnd([char[]]@("`r", "`n"))
        if ($entry -cnotmatch ('\A100(?:644|755) blob [0-9a-f]{40}\t' + [regex]::Escape($path) + '\z')) {
            throw "Candidate requires a tracked regular file: $path."
        }
        if ($CheckWorktree) {
            $expected = ($entry -split '[ \t]')[2]
            $actual = (Read-CandidateGit $ProjectRoot @('hash-object', "--path=$path", '--', $path)).Trim()
            if ($actual -cne $expected) { throw "Candidate file does not match its commit: $path." }
        }
    }
}

function Get-CandidateRemoteTag([string]$ProjectRoot, [string]$Tag) {
    $ref = "refs/tags/$Tag"
    [string]$output = Read-CandidateGit $ProjectRoot @('ls-remote', '--tags', 'origin', $ref, "$ref^{}")
    $refs = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
    if ($output.Length -gt 0) {
        foreach ($line in ($output.TrimEnd([char[]]@("`r", "`n")) -split '\r?\n')) {
            if ($line -cnotmatch '\A([0-9a-f]{40})\t(refs/tags/\S+)\z' -or
                $Matches[2] -cnotin @($ref, "$ref^{}")) { throw "Unexpected remote tag response for $Tag." }
            if (-not $refs.TryAdd($Matches[2], $Matches[1])) { throw "Duplicate remote tag response for $Tag." }
        }
    }
    if ($refs.ContainsKey("$ref^{}")) {
        if (-not $refs.ContainsKey($ref)) { throw "Remote tag $Tag has no unpeeled ref." }
        return $refs["$ref^{}"]
    }
    if ($refs.ContainsKey($ref)) { return $refs[$ref] }
    return $null
}

# Shared GitHub API transport. Detection uses only reads (including the GraphQL query).
function Invoke-CandidateGitHub([string[]]$Arguments) {
    $PSNativeCommandUseErrorActionPreference = $false
    $output = & gh api --hostname github.com @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "GitHub API request failed (exit $LASTEXITCODE): $($output -join "`n")" }
    return ($output -join "`n")
}

function Read-CandidateGitHubJson([string[]]$Arguments) {
    $raw = Invoke-CandidateGitHub $Arguments
    try { $value = ConvertFrom-Json -InputObject $raw -AsHashtable -NoEnumerate -ErrorAction Stop }
    catch { throw "Invalid GitHub release JSON: $($_.Exception.Message)" }
    return ,$value
}

function Assert-RemoteReleaseRecord($Release) {
    if ($Release -isnot [Collections.IDictionary] -or
        ($Release['id'] -isnot [long] -and $Release['id'] -isnot [int]) -or $Release['id'] -le 0 -or
        $Release['tag_name'] -isnot [string] -or [string]::IsNullOrWhiteSpace($Release['tag_name']) -or
        $Release['draft'] -isnot [bool] -or
        ($null -ne $Release['name'] -and $Release['name'] -isnot [string]) -or
        $Release['target_commitish'] -isnot [string] -or [string]::IsNullOrWhiteSpace($Release['target_commitish'])) {
        throw 'Invalid GitHub release identity.'
    }
}

function Get-CandidateRemoteRelease([string]$Repository, [string]$Tag, [string]$Commit, [AllowNull()][string]$TagTarget, [ref]$ReleaseRecords) {
    $owner, $name = $Repository.Split('/')
    # Exact pending-tag lookup also resolves drafts whose REST tag is untagged-*.
    $query = 'query($owner: String!, $name: String!, $tag: String!) { repository(owner: $owner, name: $name) { nameWithOwner release(tagName: $tag) { databaseId isDraft } } }'
    $response = Read-CandidateGitHubJson @('graphql', '--method', 'POST', '-f', "query=$query", '-f', "owner=$owner", '-f', "name=$name", '-f', "tag=$Tag")
    if ($response -isnot [Collections.IDictionary] -or $response.Contains('errors') -or
        $response['data'] -isnot [Collections.IDictionary] -or $response['data']['repository'] -isnot [Collections.IDictionary]) {
        throw "GitHub could not establish release identity for $Tag."
    }
    $metadata = $response['data']['repository']
    if ($metadata['nameWithOwner'] -isnot [string] -or $metadata['nameWithOwner'] -ine $Repository -or -not $metadata.Contains('release')) {
        throw "Invalid exact GitHub repository or release lookup for $Tag."
    }
    $exact = $metadata['release']
    if ($null -ne $exact -and ($exact -isnot [Collections.IDictionary] -or
        ($exact['databaseId'] -isnot [long] -and $exact['databaseId'] -isnot [int]) -or $exact['databaseId'] -le 0 -or $exact['isDraft'] -isnot [bool])) {
        throw "Invalid exact release lookup for $Tag."
    }

    # A successful, complete visible list distinguishes absence from failed queries and
    # exposes duplicate or disconnected draft identities. Never adopt by title.
    $pages = Read-CandidateGitHubJson @('--method', 'GET', '--paginate', '--slurp', "repos/$Repository/releases?per_page=100")
    if ($pages -isnot [array] -or $pages.Count -eq 0) { throw 'Invalid GitHub release list.' }
    $allReleases = [Collections.Generic.List[object]]::new()
    $seen = [Collections.Generic.HashSet[long]]::new()
    $matchingReleases = [Collections.Generic.List[object]]::new()
    foreach ($page in $pages) {
        if ($page -isnot [array]) { throw 'Invalid GitHub release list page.' }
        foreach ($release in $page) {
            Assert-RemoteReleaseRecord $release
            if (-not $seen.Add($release['id'])) { throw 'Duplicate GitHub release ID.' }
            $allReleases.Add($release)
            if ($release['tag_name'] -ceq $Tag -or
                ($release['draft'] -and $release['name'] -cin @("Apollo $Tag", $Tag)) -or
                ($null -ne $exact -and $release['id'] -eq $exact['databaseId'])) {
                $matchingReleases.Add($release)
            }
        }
    }
    if ($null -ne $ReleaseRecords) { $ReleaseRecords.Value = $allReleases.ToArray() }
    if ($null -eq $exact) {
        if ($matchingReleases.Count -ne 0) { throw "Release $Tag has inconsistent or ambiguous identity." }
        return $null
    }
    if ($matchingReleases.Count -ne 1) { throw "Release $Tag has missing or ambiguous identity." }
    $match = $matchingReleases[0]
    if ($match['id'] -ne $exact['databaseId'] -or $match['draft'] -ne $exact['isDraft'] -or
        ($match['tag_name'] -cne $Tag -and (-not $match['draft'] -or $match['tag_name'] -cnotmatch '\Auntagged-[0-9a-f]+\z'))) {
        throw "Release $Tag has conflicting tag or draft identity."
    }
    if ($TagTarget -cne $Commit) { throw "Release $Tag exists without a matching remote tag." }
    # GitHub ignores a branch target_commitish when the tag already exists.
    # An explicitly conflicting commit identity still requires reviewed recovery.
    if ($match['target_commitish'] -cmatch '\A[0-9a-fA-F]{40}\z' -and $match['target_commitish'] -ine $Commit) {
        throw "Release $Tag declares a conflicting commit."
    }
    return $match
}

function Get-LocalReleaseCandidate([string]$ProjectRoot, [string]$Commit, [string]$Repository) {
    $root = Resolve-RepositoryRoot $ProjectRoot
    Assert-CandidateCheckout $root $Commit $Repository
    Assert-CandidateTrackedFiles $root $Commit @('Cargo.toml', 'Cargo.lock') -CheckWorktree
    $current = (Invoke-ReleaseValidation -ProjectRoot $root).Version
    $parent = (Read-CandidateGit $root @('rev-parse', '--verify', "$Commit^1")).Trim()
    if ($parent -cnotmatch '\A[0-9a-f]{40}\z') { throw 'Candidate first parent is invalid.' }
    Assert-CandidateTrackedFiles $root $parent @('Cargo.toml')
    $previous = ConvertFrom-CargoManifestPackageInfo (Read-CandidateGit $root @('show', "${parent}:Cargo.toml"))
    if ($previous.Name -cne 'apollo') { throw 'First-parent manifest does not identify Apollo.' }
    Assert-StableReleaseVersion $previous.Version
    $comparison = ([version]$current).CompareTo([version]$previous.Version)
    if ($comparison -eq 0) {
        return [ordered]@{ candidate = $false; version = $current; sha = $Commit; reason = 'Application version is unchanged.' }
    }
    if ($comparison -lt 0) { throw "Application version decreased: $($previous.Version) -> $current." }
    $tag = "v$current"
    Assert-CandidateTrackedFiles $root $Commit @('CHANGELOG.md', "release-notes/$tag.md") -CheckWorktree
    $null = Invoke-ReleaseValidation -ProjectRoot $root -Tag $tag -RequireChangelog
    return [ordered]@{ candidate = $true; version = $current; sha = $Commit; reason = '' }
}

function Find-ReleaseCandidate([string]$ProjectRoot, [string]$Commit, [string]$Repository) {
    $root = Resolve-RepositoryRoot $ProjectRoot
    $result = Get-LocalReleaseCandidate $root $Commit $Repository
    if (-not $result.candidate) { return $result }
    $tag = "v$($result.version)"
    $target = Get-CandidateRemoteTag $root $tag
    if ($null -ne $target -and $target -cne $Commit) { throw "Remote tag $tag points to $target, not $Commit." }
    $null = Get-CandidateRemoteRelease $Repository $tag $Commit $target
    return $result
}

function Invoke-DetectReleaseCandidateCli {
    param([AllowEmptyCollection()][object[]]$Arguments)
    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        commit = @{}
        repo = @{}
        project = @{ Aliases = @('-p'); Default = (Join-Path $PSScriptRoot '..') }
    } -Positionals @('commit', 'repo')
    if ($cli['help']) {
        Write-CliHelp @'
Usage: .\scripts\detect-release-candidate.ps1 SHA REPO [options]

Arguments:
  SHA                           Exact full commit SHA of a clean checkout.
  REPO                          GitHub owner/repository (must match origin).

Options:
  -p, --project <path>          Checkout root.
  -h, --help                    Show this help.

Requires complete Git history and authenticated GitHub CLI access for a version
increase. Emits JSON and, when set, GITHUB_OUTPUT. No remote writes or CI authorization.
'@
        return
    }
    $result = Find-ReleaseCandidate ([string]$cli['project']) ([string]$cli['commit']) ([string]$cli['repo'])
    if ($env:GITHUB_OUTPUT) {
        $lines = foreach ($key in @('candidate', 'version', 'sha', 'reason')) {
            $value = if ($key -eq 'candidate') { ([string]$result[$key]).ToLowerInvariant() } else { [string]$result[$key] }
            if ($value -match '[\x00-\x1f\x7f]') { throw 'Unsafe candidate output.' }
            "$key=$value"
        }
        [IO.File]::AppendAllText($env:GITHUB_OUTPUT, (($lines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
    }
    $result | ConvertTo-Json -Compress
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-DetectReleaseCandidateCli -Arguments $args }
