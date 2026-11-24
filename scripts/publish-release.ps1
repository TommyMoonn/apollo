#requires -Version 7.0
# Adapted from Archeion (GPL-3.0-only), commit 4b711fc2af4c70e321edd8b9072527c08c13fbf2.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'detect-release-candidate.ps1')
. (Join-Path $PSScriptRoot 'verify-windows-release.ps1')

function Invoke-PublicationMutation([string]$Endpoint, [string]$Method, $Body) {
    # JSON input preserves the note verbatim and avoids native argument quoting/length limits.
    $inputFile = New-TemporaryFile
    try {
        [IO.File]::WriteAllText($inputFile.FullName, (ConvertTo-Json -InputObject $Body -Depth 8 -Compress), [Text.UTF8Encoding]::new($false))
        return Read-CandidateGitHubJson @($Endpoint, '--method', $Method, '--input', $inputFile.FullName)
    } finally { Remove-Item -LiteralPath $inputFile.FullName -Force }
}

function Get-PublicationHeaders([string]$Accept) {
    if ([string]::IsNullOrWhiteSpace($env:GH_TOKEN)) { throw 'Publication requires the authorized workflow token.' }
    return @{ Authorization = "Bearer $env:GH_TOKEN"; Accept = $Accept; 'X-GitHub-Api-Version' = '2022-11-28' }
}

function Send-PublicationAsset([string]$Repository, [long]$ReleaseId, [string]$Path) {
    $ProgressPreference = 'SilentlyContinue'
    $name = [Uri]::EscapeDataString([IO.Path]::GetFileName($Path))
    # Upload to the resolved numeric ID, not a mutable tag lookup. POST refuses a duplicate name.
    $uri = "https://uploads.github.com/repos/$Repository/releases/$ReleaseId/assets?name=$name"
    $null = Invoke-WebRequest -Uri $uri -Method Post -Headers (Get-PublicationHeaders 'application/vnd.github+json') `
        -ContentType 'application/octet-stream' -InFile $Path -MaximumRedirection 0 -TimeoutSec 600
}

function Get-PublicationAssetDigest([string]$Repository, [long]$AssetId) {
    $ProgressPreference = 'SilentlyContinue'
    $temporary = New-TemporaryFile
    try {
        # PowerShell strips authorization on redirects by default. Do not forward the token to storage hosts.
        $null = Invoke-WebRequest -Uri "https://api.github.com/repos/$Repository/releases/assets/$AssetId" `
            -Headers (Get-PublicationHeaders 'application/octet-stream') -OutFile $temporary.FullName -MaximumRedirection 5 -TimeoutSec 600
        return Get-ReleaseFileRecord $temporary.FullName
    } finally { Remove-Item -LiteralPath $temporary.FullName -Force }
}

function Assert-PublicationOrder([object[]]$Records, [string]$Version) {
    foreach ($record in $Records) {
        if ($record.draft) { continue }
        if ($record.prerelease -isnot [bool]) { throw 'Invalid published release prerelease status.' }
        if ($record.prerelease -or $record.tag_name -cnotmatch '\Av(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z') { continue }
        $publishedVersion = $record.tag_name.Substring(1)
        Assert-StableReleaseVersion $publishedVersion
        if ([version]$publishedVersion -gt [version]$Version) {
            throw "A higher stable version ($publishedVersion) is already published; refusing $Version."
        }
    }
}

function Assert-PublicationRelease($Release, [long]$ReleaseId, [string]$Tag, [string]$Commit, [string]$Body) {
    Assert-RemoteReleaseRecord $Release
    if ($Release.id -ne $ReleaseId -or $Release.name -cne "Apollo $Tag" -or $Release.body -isnot [string] -or
        $Release.body -cne $Body -or $Release.prerelease -isnot [bool] -or $Release.prerelease -or
        ($Release.tag_name -cne $Tag -and (-not $Release.draft -or $Release.tag_name -cnotmatch '\Auntagged-[0-9a-f]+\z')) -or
        ($Release.target_commitish -cmatch '\A[0-9a-fA-F]{40}\z' -and $Release.target_commitish -ine $Commit)) {
        throw "Release $Tag has conflicting identity, title, body or prerelease status."
    }
}

function Get-VerifiedPublicationAssets($Release, [string]$Repository, $Expected) {
    if ($Release.assets -isnot [array]) { throw 'Invalid remote asset list.' }
    $verified = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $ids = [Collections.Generic.HashSet[long]]::new()
    foreach ($asset in $Release.assets) {
        if ($asset -isnot [Collections.IDictionary] -or $asset['name'] -isnot [string] -or
            $Expected.Keys -cnotcontains $asset['name'] -or -not $verified.Add($asset['name']) -or
            ($asset['id'] -isnot [long] -and $asset['id'] -isnot [int]) -or $asset['id'] -le 0 -or -not $ids.Add($asset['id']) -or
            ($asset['size'] -isnot [long] -and $asset['size'] -isnot [int]) -or $asset['size'] -ne $Expected[$asset['name']].size -or
            $asset['state'] -cne 'uploaded') { throw 'Unexpected, incomplete or conflicting remote release asset.' }
        $record = $Expected[$asset['name']]
        $digest = $asset['digest']
        if ($null -eq $digest -or $digest -ceq '') {
            Assert-ReleaseRecord $record (Get-PublicationAssetDigest $Repository $asset['id']) $asset['name']
        } elseif ($digest -isnot [string] -or $digest -cne "sha256:$($record.sha256)") {
            throw "Remote release asset digest mismatch: $($asset['name'])."
        }
    }
    return ,$verified
}

function Get-PublicationRemoteState([string]$ProjectRoot, [string]$Repository, [string]$Tag, [string]$Commit) {
    $target = Get-CandidateRemoteTag $ProjectRoot $Tag
    if ($target -and $target -cne $Commit) { throw "Remote tag $Tag points to $target, not $Commit." }
    $records = @()
    $release = Get-CandidateRemoteRelease $Repository $Tag $Commit $target -ReleaseRecords ([ref]$records)
    return [pscustomobject]@{ Target = $target; Release = $release; Records = $records }
}

function Invoke-ReleasePublication {
    param([string]$ProjectRoot, [string]$Commit, [string]$Version, [string]$Repository, [string]$ArtifactsDirectory)
    $root = Resolve-RepositoryRoot $ProjectRoot
    Assert-StableReleaseVersion $Version
    $candidate = Get-LocalReleaseCandidate $root $Commit $Repository
    if (-not $candidate.candidate -or $candidate.version -cne $Version) { throw 'Publication requires an exact version-increase candidate.' }
    $tag = "v$Version"
    # Candidate validation already checked the note; publish the Git blob rather than
    # worktree text, which may have platform-specific line endings.
    $body = Read-CandidateGit $root @('show', "${Commit}:release-notes/$tag.md")
    $directory = Resolve-ReleasePath -ProjectRoot $root -Path $ArtifactsDirectory -OptionName '--artifacts-dir'
    $provenance = Assert-WindowsReleaseArtifacts -ProjectRoot $root -ArtifactsDirectory $directory -ExpectedVersion $Version -ExpectedSha $Commit -PassThru
    $expected = $provenance.assets

    $state = Get-PublicationRemoteState $root $Repository $tag $Commit
    $release = $state.Release
    if ($null -ne $release -and -not $release.draft) {
        $releaseId = $release.id
        Assert-PublicationRelease $release $releaseId $tag $Commit $body
        $verified = Get-VerifiedPublicationAssets $release $Repository $expected
        if ($verified.Count -ne $expected.Count) { throw "Published release $tag is incomplete." }
        Write-CliSuccess "Release $tag already published and verified; no writes required."
        return
    }
    Assert-PublicationOrder $state.Records $Version

    if (-not $state.Target) {
        # Create a lightweight tag only. If a race or lost response occurs, accept only a matching remote ref.
        $creationError = $null
        try { $null = Invoke-PublicationMutation "repos/$Repository/git/refs" POST @{ ref = "refs/tags/$tag"; sha = $Commit } }
        catch { $creationError = $_.Exception.Message; Write-CliWarning 'Tag creation did not return success; checking the remote ref.' }
        $state = Get-PublicationRemoteState $root $Repository $tag $Commit
        if ($state.Target -cne $Commit) { throw "Could not establish remote tag $tag at $Commit. $creationError" }
        Assert-PublicationOrder $state.Records $Version
        $release = $state.Release
    }
    if ($null -eq $release) {
        if ((Get-CandidateRemoteTag $root $tag) -cne $Commit) { throw "Remote tag $tag changed before draft creation." }
        $created = $null
        $creationError = $null
        try {
            $created = Invoke-PublicationMutation "repos/$Repository/releases" POST @{
                tag_name = $tag; target_commitish = $Commit; name = "Apollo $tag"; body = $body; draft = $true; prerelease = $false
            }
        } catch { $creationError = $_.Exception.Message; Write-CliWarning 'Draft creation did not return success; checking the exact remote release.' }
        $state = Get-PublicationRemoteState $root $Repository $tag $Commit
        if ($state.Target -cne $Commit -or $null -eq $state.Release) { throw "Could not establish draft release $tag. $creationError" }
        Assert-PublicationOrder $state.Records $Version
        $release = $state.Release
        if ($null -ne $created) {
            Assert-PublicationRelease $created $release.id $tag $Commit $body
            if (-not $created.draft) { throw "Unexpected published response to draft creation for $tag." }
        }
    }
    $releaseId = $release.id
    Assert-PublicationRelease $release $releaseId $tag $Commit $body
    $verified = Get-VerifiedPublicationAssets $release $Repository $expected
    if (-not $release.draft) { throw "Release $tag changed to published during preparation; rerun to verify it." }

    foreach ($name in $expected.Keys) {
        if ($verified.Contains($name)) { continue }
        $path = Join-Path $directory $name
        Assert-ReleaseRecord $expected[$name] (Get-ReleaseFileRecord $path) $name
        # A failed or interrupted upload is not overwritten. The next run rechecks remote bytes.
        if ((Get-CandidateRemoteTag $root $tag) -cne $Commit) { throw "Remote tag $tag changed before upload." }
        Send-PublicationAsset $Repository $releaseId $path
        $state = Get-PublicationRemoteState $root $Repository $tag $Commit
        if ($state.Target -cne $Commit -or $null -eq $state.Release) { throw "Release $tag lost its matching tag or draft." }
        Assert-PublicationOrder $state.Records $Version
        $release = $state.Release
        Assert-PublicationRelease $release $releaseId $tag $Commit $body
        if (-not $release.draft) { throw "Release $tag is no longer a draft." }
        $verified = Get-VerifiedPublicationAssets $release $Repository $expected
        if (-not $verified.Contains($name)) { throw "Upload did not produce a verified remote asset: $name." }
    }

    # Re-resolve the exact tag and draft after uploads, and recheck ordering before publication.
    $state = Get-PublicationRemoteState $root $Repository $tag $Commit
    if ($state.Target -cne $Commit -or $null -eq $state.Release) { throw "Release $tag lost its matching tag or draft." }
    Assert-PublicationOrder $state.Records $Version
    Assert-PublicationRelease $state.Release $releaseId $tag $Commit $body
    $verified = Get-VerifiedPublicationAssets $state.Release $Repository $expected
    if (-not $state.Release.draft -or $verified.Count -ne $expected.Count) { throw "Release $tag is not a complete verified draft." }
    if ((Get-CandidateRemoteTag $root $tag) -cne $Commit) { throw "Remote tag $tag changed before publication." }
    $published = Invoke-PublicationMutation "repos/$Repository/releases/$releaseId" PATCH @{ tag_name = $tag; draft = $false }
    Assert-PublicationRelease $published $releaseId $tag $Commit $body
    if ($published.draft) { throw "Release $tag remained a draft after publication." }

    $state = Get-PublicationRemoteState $root $Repository $tag $Commit
    if ($state.Target -cne $Commit -or $null -eq $state.Release) { throw "Published release $tag could not be confirmed." }
    Assert-PublicationRelease $state.Release $releaseId $tag $Commit $body
    $verified = Get-VerifiedPublicationAssets $state.Release $Repository $expected
    if ($state.Release.draft -or $verified.Count -ne $expected.Count) { throw "Published release $tag could not be verified." }
    Write-CliSuccess "Release $tag published and verified."
}

function Invoke-PublishReleaseCli {
    param([AllowEmptyCollection()][object[]]$Arguments)
    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        project = @{ Default = (Join-Path $PSScriptRoot '..') }
        commit = @{}; version = @{}; repo = @{}; 'artifacts-dir' = @{}
    }
    if ($cli.help) {
        Write-CliHelp @'
Usage: publish-release.ps1 --commit SHA --version X.Y.Z --repo OWNER/REPO --artifacts-dir PATH [--project PATH]

For the authorized GitHub Actions publication job only. Requires a clean exact
candidate checkout, complete history, independently verified artifacts and GH_TOKEN.
No local publication recipe. The workflow is responsible for CI and artifact authorization.
'@
        return
    }
    if ($env:GITHUB_ACTIONS -cne 'true' -or [string]::IsNullOrWhiteSpace($env:GH_TOKEN) -or
        [string]::IsNullOrWhiteSpace($env:GITHUB_REPOSITORY) -or $env:GITHUB_REPOSITORY -ine $cli.repo) {
        throw 'Publication requires the authorized GitHub Actions job and repository token.'
    }
    Invoke-ReleasePublication -ProjectRoot $cli.project -Commit $cli.commit -Version $cli.version -Repository $cli.repo -ArtifactsDirectory $cli['artifacts-dir']
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-PublishReleaseCli -Arguments $args }
