#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepositoryPathComparison {
    if ($IsWindows) {
        return [System.StringComparison]::OrdinalIgnoreCase
    }

    return [System.StringComparison]::Ordinal
}

function Test-RepositoryFileSystemRoot {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $false
    }

    try {
        $fullPath = [System.IO.Path]::GetFullPath($Path)
        $rootPath = [System.IO.Path]::GetPathRoot($fullPath)
    }
    catch {
        return $false
    }

    if ([string]::IsNullOrWhiteSpace($rootPath)) {
        return $false
    }

    $trimChars = [char[]]@('\', '/')
    $normalizedPath = $fullPath.TrimEnd($trimChars)
    $normalizedRoot = $rootPath.TrimEnd($trimChars)

    return $normalizedPath.Equals($normalizedRoot, (Get-RepositoryPathComparison))
}

function Resolve-RepositoryRoot {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $fullPath = [System.IO.Path]::GetFullPath($Path).TrimEnd([char[]]@('\', '/'))
    if (-not (Test-Path -LiteralPath $fullPath -PathType Container)) {
        throw "Project root does not exist: $fullPath"
    }

    return $fullPath
}

function Get-RepositorySlug {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot
    )

    $projectName = Split-Path $ProjectRoot -Leaf
    $slug = ($projectName.ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')

    if ([string]::IsNullOrWhiteSpace($slug)) {
        throw "Could not derive a project slug from '$projectName'."
    }

    return $slug
}

function Resolve-RepositoryRelativePath {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [Parameter(Mandatory)]
        [string]$RelativePath,

        [string]$Operation = 'access'
    )

    $directorySeparator = [System.IO.Path]::DirectorySeparatorChar
    $normalized = $RelativePath.Trim().Replace([char]'/', $directorySeparator).Replace([char]'\', $directorySeparator)

    if ([string]::IsNullOrWhiteSpace($normalized)) {
        throw "Encountered an empty project-relative path."
    }

    if ([System.IO.Path]::IsPathRooted($normalized)) {
        throw "Absolute paths are not allowed: $RelativePath"
    }

    if ($normalized -match '(^|[\\/])\.\.([\\/]|$)') {
        throw "Parent-directory traversal is not allowed: $RelativePath"
    }

    if ($normalized -match '(^|[\\/])\.git([\\/]|$)') {
        throw "Refusing to $Operation .git content: $RelativePath"
    }

    $root = [System.IO.Path]::GetFullPath($ProjectRoot).TrimEnd([char[]]@('\', '/'))
    $comparison = Get-RepositoryPathComparison
    $rootPrefix = "$root$directorySeparator"
    $fullPath = [System.IO.Path]::GetFullPath((Join-Path $root $normalized))

    if (-not $fullPath.StartsWith($rootPrefix, $comparison)) {
        throw "Path resolves outside the project root: $RelativePath"
    }

    return $fullPath
}

function Invoke-RepositoryGit {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [Parameter(Mandatory)]
        [string[]]$Arguments,

        [switch]$AllowFailure
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = 'git'
    $startInfo.WorkingDirectory = $ProjectRoot
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [System.Text.UTF8Encoding]::new($false)

    foreach ($argument in $Arguments) {
        [void]$startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo

    try {
        if (-not $process.Start()) {
            throw 'Unable to start Git.'
        }

        # Read stderr asynchronously so a noisy child cannot block while stdout is drained.
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $stdout = $process.StandardOutput.ReadToEnd()
        $process.WaitForExit()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $exitCode = $process.ExitCode

        if ($exitCode -ne 0 -and -not $AllowFailure) {
            throw "Git failed: git $($Arguments -join ' ')`n$stderr"
        }

        return [pscustomobject]@{
            ExitCode = $exitCode
            StdOut = $stdout
            StdErr = $stderr
        }
    }
    finally {
        $process.Dispose()
    }
}

function Assert-GitRepository {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot
    )

    $inside = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('rev-parse', '--is-inside-work-tree') -AllowFailure
    if ($inside.ExitCode -ne 0 -or $inside.StdOut.Trim() -ne 'true') {
        throw "Project root is not inside a Git working tree: $ProjectRoot"
    }

    $head = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @('rev-parse', '--verify', 'HEAD') -AllowFailure
    if ($head.ExitCode -ne 0) {
        throw "Git repository does not have a HEAD commit: $ProjectRoot"
    }
}

function Get-GitNulValues {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    $result = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments $Arguments
    return @($result.StdOut.Split([char]0, [System.StringSplitOptions]::RemoveEmptyEntries))
}

function Get-GitTrackedChanges {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [string]$BaseCommit = 'HEAD'
    )

    $result = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @(
        'diff',
        $BaseCommit,
        '--name-status',
        '-z',
        '--find-renames',
        '--'
    )

    $tokens = $result.StdOut.Split([char]0, [System.StringSplitOptions]::RemoveEmptyEntries)
    $changes = [System.Collections.Generic.List[object]]::new()

    for ($index = 0; $index -lt $tokens.Length;) {
        $statusToken = $tokens[$index++]
        if ([string]::IsNullOrEmpty($statusToken)) {
            continue
        }

        $status = [string]$statusToken[0]
        if ($status -in @('R', 'C')) {
            if (($index + 1) -ge $tokens.Length) {
                throw 'Unexpected Git rename/copy output.'
            }

            $oldPath = $tokens[$index++]
            $newPath = $tokens[$index++]
            $changes.Add([pscustomobject]@{
                Status = $status
                Path = $newPath
                OldPath = $oldPath
            })
            continue
        }

        if ($index -ge $tokens.Length) {
            throw 'Unexpected Git diff output.'
        }

        $changes.Add([pscustomobject]@{
            Status = $status
            Path = $tokens[$index++]
            OldPath = $null
        })
    }

    return @($changes)
}

function Get-GitUntrackedPaths {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot
    )

    return @(Get-GitNulValues -ProjectRoot $ProjectRoot -Arguments @(
        'ls-files', '--others', '--exclude-standard', '-z'
    ))
}

function Test-GitTrackedDirty {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot
    )

    $result = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @(
        'status', '--porcelain', '--untracked-files=no'
    )

    return -not [string]::IsNullOrWhiteSpace($result.StdOut)
}

function Get-ChatGptImportBase {
    $localAppData = $env:LOCALAPPDATA
    if ([string]::IsNullOrWhiteSpace($localAppData)) {
        $localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    }
    if ([string]::IsNullOrWhiteSpace($localAppData)) {
        $localAppData = [System.IO.Path]::GetTempPath()
    }

    return Join-Path $localAppData 'ChatGPTImports'
}

# Transfer paths use a portable subset of Windows filenames. Never normalize an
# ambiguous spelling into a different target, or follow a link outside the tree.
function Assert-TransferUnlinked {
    param([string]$Path)
    for ($current = $Path; $current; $current = [IO.Path]::GetDirectoryName($current)) {
        $item = Get-Item -LiteralPath $current -Force -ErrorAction SilentlyContinue
        if ($null -ne $item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Symbolic links and reparse points are not supported: $current"
        }
        if ($current -ne $Path -and $null -ne $item -and -not $item.PSIsContainer) {
            throw "Parent path is not a directory: $current"
        }
    }
}

function Assert-TransferRoot {
    param([string]$Path)
    if (Test-RepositoryFileSystemRoot -Path $Path) { throw 'Transfer cannot target a filesystem root.' }
    Assert-TransferUnlinked -Path $Path
}

function Test-TransferMetadata {
    param([string]$Path)
    return $Path.Replace('\', '/') -match '(?i)(^|/)(?:\.chatgpt-delete-manifest\.txt|\.[^/]+-change-package\.json|EXPORT_MANIFEST\.json|\.apollo-import-backup\.json)(/|$)'
}

function Test-TransferExcluded {
    param([string]$Path)
    $path = $Path.Replace('\', '/')
    return ($path -match '^(?:\.plans|\.planning|\.project|\.agent-work|\.agents/skills|\.dev|\.scratch|target|artifacts|test-results|coverage)(?:/|$)' -or
        $path -match '(?:^|/)(?:\.env(?:\.[^/]*)?|[^/]+\.(?:pem|key|pfx|p12|zip|log|tmp)|\.DS_Store|Thumbs\.db|Desktop\.ini)(?:/|$)')
}

function ConvertTo-TransferPath {
    param([string]$Path)
    $normalized = $Path.Replace('\', '/')
    if ($normalized -match '(^/|^[A-Za-z]:)') { throw "Absolute paths are not allowed: $Path" }
    if ($normalized -match '(^|/)\.\.(/|$)') { throw "Parent-directory traversal is not allowed: $Path" }
    foreach ($part in $normalized.Split('/')) {
        if (-not $part -or $part -eq '.' -or $part -ne $part.Trim() -or $part.EndsWith('.') -or $part -match '[\x00-\x1f<>:"|?*]' -or
            $part -match '^(?:CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])(?:\.|$)') { throw "Unsafe path spelling: $Path" }
        if ($part -ieq '.git') { throw "Refusing to access .git content: $Path" }
    }
    return $normalized
}

function Assert-TransferPathSpelling {
    param([string]$Root, [string]$Path)
    if ($IsWindows) {
        # Short-name aliases must not bypass protected paths or collision checks.
        for ($current = $Path; -not $current.Equals($Root, (Get-RepositoryPathComparison)); $current = [IO.Path]::GetDirectoryName($current)) {
            if (Test-Path -LiteralPath $current) {
                $parent = [IO.Path]::GetDirectoryName($current)
                $name = [IO.Path]::GetFileName($current)
                if (-not @(Get-ChildItem -LiteralPath $parent -Filter $name -Force | Where-Object { $_.Name -ieq $name }).Count) { throw "Unsafe filesystem alias: $current" }
            }
        }
    }
}

function Resolve-TransferPath {
    param([string]$Root, [string]$Path)
    $normalized = ConvertTo-TransferPath -Path $Path
    if (Test-TransferMetadata -Path $normalized) { throw "Reserved transfer metadata path: $Path" }
    if (Test-TransferExcluded -Path $normalized) { throw "Protected local or generated path: $Path" }
    $full = Resolve-RepositoryRelativePath -ProjectRoot $Root -RelativePath $normalized -Operation 'transfer'
    Assert-TransferUnlinked -Path $full
    Assert-TransferPathSpelling -Root $Root -Path $full
    return $full
}

function Assert-TransferDisjoint {
    param([AllowEmptyCollection()][string[]]$Paths)
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $spelling = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($path in $Paths) {
        if (-not $seen.Add($path)) { throw "Duplicate or conflicting target path: $path" }
        for ($current = $path; $current; $current = $current -replace '/[^/]+$','') {
            if ($spelling.ContainsKey($current) -and $spelling[$current] -cne $current) { throw "Case-colliding target path: $path" }
            $spelling[$current] = $current
            if (-not $current.Contains('/')) { break }
        }
    }
    foreach ($path in $Paths) {
        $parent = $path
        while ($parent.Contains('/')) {
            $parent = $parent -replace '/[^/]+$',''
            if ($seen.Contains($parent)) { throw "Overlapping target paths: $parent and $path" }
        }
    }
}

function Get-ApolloImportBase {
    return Join-Path (Split-Path (Get-ChatGptImportBase) -Parent) 'Apollo/ChatGPTImports'
}

function Get-TransferTree {
    param([string]$Path)
    Assert-TransferUnlinked -Path $Path
    $tree = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
    if (-not (Test-Path -LiteralPath $Path)) { return ,$tree }
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($Path)
    while ($pending.Count) {
        $current = $pending.Pop()
        Assert-TransferUnlinked -Path $current
        $relative = [IO.Path]::GetRelativePath($Path, $current).Replace('\', '/')
        if ($relative -ne '.') {
            [void](ConvertTo-TransferPath -Path $relative)
            if (Test-TransferMetadata -Path $relative) { throw "Protected metadata inside directory operation: $current" }
        }
        if (Test-Path -LiteralPath $current -PathType Container) {
            $tree.Add($relative, 'directory')
            foreach ($child in (Get-ChildItem -LiteralPath $current -Force)) { $pending.Push($child.FullName) }
        } else {
            $tree.Add($relative, ('file:' + (Get-FileHash -LiteralPath $current -Algorithm SHA256).Hash))
        }
    }
    return ,$tree
}

function Test-TransferTreeSubset {
    param($Actual, $Expected)
    foreach ($key in $Actual.Keys) {
        if (-not $Expected.ContainsKey($key) -or $Actual[$key] -cne $Expected[$key]) { return $false }
    }
    return $true
}

function Get-TransferFileState {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return 'absent' }
    if (Test-Path -LiteralPath $Path -PathType Container) { return 'directory' }
    return 'file:' + (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Copy-TransferFile {
    param([string]$Source, [string]$Target)
    # Publish a complete file or leave the previous target untouched on copy failure.
    $temp = Join-Path (Split-Path $Target -Parent) ('.apollo-transfer-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::Copy($Source, $temp)
        [IO.File]::Move($temp, $Target, $true)
    } finally {
        if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Force }
    }
}

function Write-TransferJson {
    param([string]$Path, $Value)
    $temp = "$Path.tmp"
    try {
        [IO.File]::WriteAllText($temp, ($Value | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temp, $Path, $false)
    } finally {
        if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Force }
    }
}

function Get-TransferTreeDigest {
    param($Tree)
    $keys = [string[]]@($Tree.Keys)
    [Array]::Sort($keys, [StringComparer]::Ordinal)
    $lines = @(foreach ($key in $keys) { "$key`n$($Tree[$key])" })
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($lines -join "`n")))).Replace('-', '') }
    finally { $sha.Dispose() }
}
