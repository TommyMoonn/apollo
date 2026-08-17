#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$testFiles = @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.Tests.ps1' -File | Sort-Object Name)
if ($testFiles.Count -eq 0) {
    throw "No PowerShell test files were found under '$PSScriptRoot'."
}

foreach ($testFile in $testFiles) {
    & $testFile.FullName
}
