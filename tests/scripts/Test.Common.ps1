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
