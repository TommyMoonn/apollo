#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$scriptFiles = @(
    Get-ChildItem -LiteralPath (Join-Path $projectRoot 'scripts') -Filter '*.ps1' -File
    Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File
) | Sort-Object FullName -Unique

$parseFailures = [System.Collections.Generic.List[string]]::new()
foreach ($scriptFile in $scriptFiles) {
    $tokens = $null
    $errors = $null
    [System.Management.Automation.Language.Parser]::ParseFile(
        $scriptFile.FullName,
        [ref]$tokens,
        [ref]$errors
    ) | Out-Null

    foreach ($parseError in @($errors)) {
        $relativePath = [System.IO.Path]::GetRelativePath($projectRoot, $scriptFile.FullName)
        $parseFailures.Add(
            "${relativePath}:$($parseError.Extent.StartLineNumber):$($parseError.Extent.StartColumnNumber) $($parseError.Message)"
        )
    }
}

if ($parseFailures.Count -gt 0) {
    throw "PowerShell syntax validation failed:`n$($parseFailures -join [Environment]::NewLine)"
}

$testFiles = @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.Tests.ps1' -File | Sort-Object Name)
if ($testFiles.Count -eq 0) {
    throw "No PowerShell test files were found under '$PSScriptRoot'."
}

$pwshPath = (Get-Process -Id $PID).Path
$failedSuites = [System.Collections.Generic.List[string]]::new()

foreach ($testFile in $testFiles) {
    Write-Host ""
    Write-Host "=== $($testFile.Name) ==="
    & $pwshPath -NoLogo -NoProfile -File $testFile.FullName
    if ($LASTEXITCODE -ne 0) {
        $failedSuites.Add($testFile.Name)
    }
}

if ($failedSuites.Count -gt 0) {
    throw "PowerShell test suites failed: $($failedSuites -join ', ')"
}

Write-Host ""
Write-Host "PowerShell tooling: all $($testFiles.Count) suites passed"
