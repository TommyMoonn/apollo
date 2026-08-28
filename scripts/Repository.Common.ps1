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
        [string]$ProjectRoot
    )

    $result = Invoke-RepositoryGit -ProjectRoot $ProjectRoot -Arguments @(
        'diff',
        'HEAD',
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
