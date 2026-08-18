#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:Passed = 0
$script:Failed = 0

function Assert-True {
    param(
        [Parameter(Mandatory)]
        [bool]$Condition,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Assert-Equal {
    param(
        [AllowNull()]
        $Expected,

        [AllowNull()]
        $Actual,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if ($Expected -is [System.Array] -or $Actual -is [System.Array]) {
        $expectedArray = @($Expected)
        $actualArray = @($Actual)
        if ($expectedArray.Count -ne $actualArray.Count) {
            throw "$Message Expected count $($expectedArray.Count), got $($actualArray.Count)."
        }

        for ($index = 0; $index -lt $expectedArray.Count; $index++) {
            if ($expectedArray[$index] -ne $actualArray[$index]) {
                throw "$Message Difference at index $index: expected '$($expectedArray[$index])', got '$($actualArray[$index])'."
            }
        }
        return
    }

    if ($Expected -ne $Actual) {
        throw "$Message Expected '$Expected', got '$Actual'."
    }
}

function Assert-Throws {
    param(
        [Parameter(Mandatory)]
        [scriptblock]$ScriptBlock,

        [Parameter(Mandatory)]
        [string]$MessagePattern
    )

    try {
        & $ScriptBlock
    }
    catch {
        if ($_.Exception.Message -notlike $MessagePattern) {
            throw "Expected error matching '$MessagePattern', got '$($_.Exception.Message)'."
        }
        return
    }

    throw "Expected command to throw an error matching '$MessagePattern'."
}

function Invoke-TestCase {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [scriptblock]$Body
    )

    try {
        & $Body
        $script:Passed++
        Write-Host "PASS $Name"
    }
    catch {
        $script:Failed++
        Write-Host "FAIL $Name"
        Write-Host "     $($_.Exception.Message)"
    }
}

function Complete-TestRun {
    Write-Host ""
    Write-Host "PowerShell tests: $script:Passed passed, $script:Failed failed"

    if ($script:Failed -gt 0) {
        exit 1
    }
}

function Assert-False {
    param(
        [Parameter(Mandatory)]
        [bool]$Condition,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if ($Condition) {
        throw $Message
    }
}

function Assert-Contains {
    param(
        [Parameter(Mandatory)]
        [string]$Text,

        [Parameter(Mandatory)]
        [string]$Expected,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $Text.Contains($Expected, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Message Expected text to contain '$Expected'."
    }
}

function Assert-PathExists {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "$Message Missing path: $Path"
    }
}

function Assert-PathMissing {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if (Test-Path -LiteralPath $Path) {
        throw "$Message Unexpected path: $Path"
    }
}

function New-TestDirectory {
    param([string]$Name = 'apollo-test')

    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("$Name-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    return $root
}

function New-TestGitRepository {
    param([string]$Name = 'apollo-test-repo')

    $root = New-TestDirectory -Name $Name
    & git -C $root init --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Failed to initialize disposable Git repository.' }
    & git -C $root config user.email 'apollo-tests@example.invalid'
    & git -C $root config user.name 'Apollo Tests'
    Set-Content -LiteralPath (Join-Path $root 'seed.txt') -Value 'seed' -Encoding utf8
    & git -C $root add seed.txt
    & git -C $root commit --quiet -m 'test: seed repository'
    if ($LASTEXITCODE -ne 0) { throw 'Failed to create disposable Git repository HEAD.' }
    return $root
}

function Invoke-ScriptCapture {
    param(
        [Parameter(Mandatory)][string]$ScriptPath,
        [object[]]$Arguments = @()
    )

    $captured = @(& $ScriptPath @Arguments 6>&1)
    return ($captured | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
}

function New-TestZip {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][hashtable]$Entries
    )

    Add-Type -AssemblyName System.IO.Compression
    $parent = Split-Path $Path -Parent
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force }

    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::CreateNew)
    try {
        $archive = [System.IO.Compression.ZipArchive]::new($stream, [System.IO.Compression.ZipArchiveMode]::Create, $false)
        try {
            foreach ($name in ($Entries.Keys | Sort-Object)) {
                $entry = $archive.CreateEntry([string]$name)
                $writer = [System.IO.StreamWriter]::new($entry.Open(), [System.Text.UTF8Encoding]::new($false))
                try { $writer.Write([string]$Entries[$name]) } finally { $writer.Dispose() }
            }
        }
        finally { $archive.Dispose() }
    }
    finally { $stream.Dispose() }

    return $Path
}

function Get-ZipEntryNames {
    param([Parameter(Mandatory)][string]$Path)

    Add-Type -AssemblyName System.IO.Compression
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Path)
    try {
        return @($archive.Entries | ForEach-Object { $_.FullName })
    }
    finally { $archive.Dispose() }
}

function Get-ZipEntryText {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$EntryName
    )

    Add-Type -AssemblyName System.IO.Compression
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $entry = $archive.GetEntry($EntryName)
        if ($null -eq $entry) { throw "ZIP entry not found: $EntryName" }
        $reader = [System.IO.StreamReader]::new($entry.Open())
        try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
    }
    finally { $archive.Dispose() }
}
