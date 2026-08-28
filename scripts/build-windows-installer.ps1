#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'stage-windows-release.ps1')
. (Join-Path $PSScriptRoot 'check-release.ps1')

function Resolve-NsisCompilerPath {
    param(
        [Parameter(Mandatory)]
        [string]$ProjectRoot,

        [AllowNull()]
        $Value
    )

    if ($null -ne $Value) {
        $compiler = [string]$Value
        if ([string]::IsNullOrWhiteSpace($compiler)) {
            throw "Option '--makensis' cannot be empty."
        }

        if ([System.IO.Path]::IsPathRooted($compiler) -or $compiler.Contains('\') -or $compiler.Contains('/')) {
            $resolved = Resolve-ReleasePath -ProjectRoot $ProjectRoot -Path $compiler -OptionName '--makensis'
            if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
                throw "NSIS compiler does not exist: $resolved"
            }
            return $resolved
        }

        $command = Get-Command $compiler -CommandType Application, ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -eq $command) {
            throw "NSIS compiler '$compiler' was not found."
        }
        return $command.Source
    }

    $command = Get-Command 'makensis.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $command) {
        return $command.Source
    }

    if ($IsWindows) {
        foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
            if ([string]::IsNullOrWhiteSpace($base)) {
                continue
            }
            $candidate = Join-Path $base 'NSIS/makensis.exe'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                return [System.IO.Path]::GetFullPath($candidate)
            }
        }
    }

    throw "NSIS compiler was not found. Install NSIS or pass --makensis <path-to-makensis.exe>."
}

function Assert-InstallerSourceFile {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Description
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Description does not exist: $Path"
    }
}

function Invoke-BuildWindowsInstallerCli {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Arguments
    )

    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
        executable = @{ Aliases = @('-e', '-Executable') }
        makensis = @{ Aliases = @('-m', '-Makensis') }
        output = @{ Aliases = @('-o', '-Output') }
    }

    if ($cli['help']) {
        Write-CliHelp @'
Usage: .\scripts\build-windows-installer.ps1 [options]

Options:
  -p, --project <path>          Project root. Defaults to this repository.
  -e, --executable <path>       Built apollo.exe. Defaults to the Windows x64 release target.
  -m, --makensis <path>         NSIS makensis compiler. Defaults to makensis.exe on PATH/standard install paths.
  -o, --output <path>           Installer file. Defaults to artifacts/windows-installer/Apollo-Setup-x64.exe.
  -h, --help                    Show this help.

Relative executable, compiler, and output paths resolve from the project root.
The output filename must be exactly Apollo-Setup-x64.exe.
'@
        return
    }

    $projectRoot = Resolve-RepositoryRoot -Path ([string]$cli['project'])
    $executableValue = if ($null -eq $cli['executable']) {
        'target/x86_64-pc-windows-msvc/release/apollo.exe'
    }
    else {
        [string]$cli['executable']
    }
    $outputValue = if ($null -eq $cli['output']) {
        'artifacts/windows-installer/Apollo-Setup-x64.exe'
    }
    else {
        [string]$cli['output']
    }

    $executablePath = Resolve-ReleasePath -ProjectRoot $projectRoot -Path $executableValue -OptionName '--executable'
    $outputPath = Resolve-ReleasePath -ProjectRoot $projectRoot -Path $outputValue -OptionName '--output'
    $outputName = [System.IO.Path]::GetFileName($outputPath)
    if (-not $outputName.Equals('Apollo-Setup-x64.exe', [System.StringComparison]::Ordinal)) {
        throw "Installer output must be named exactly 'Apollo-Setup-x64.exe', found '$outputName'."
    }
    if (Test-Path -LiteralPath $outputPath -PathType Container) {
        throw "Installer output must be a file path: $outputPath"
    }

    $release = Invoke-ReleaseValidation -ProjectRoot $projectRoot
    Assert-WindowsX64ApolloExecutable -Path $executablePath

    $licensePath = Join-Path $projectRoot 'LICENSE'
    $noticesPath = Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md'
    $nsiPath = Join-Path $projectRoot 'installer/windows/apollo.nsi'
    Assert-InstallerSourceFile -Path $licensePath -Description 'Project license'
    Assert-InstallerSourceFile -Path $noticesPath -Description 'Third-party notices'
    Assert-InstallerSourceFile -Path $nsiPath -Description 'NSIS installer definition'

    $compilerPath = Resolve-NsisCompilerPath -ProjectRoot $projectRoot -Value $cli['makensis']
    $comparison = Get-RepositoryPathComparison
    if ($outputPath.Equals([System.IO.Path]::GetFullPath($compilerPath), $comparison)) {
        throw "Installer output must not overwrite the NSIS compiler: $outputPath"
    }

    $outputParent = Split-Path $outputPath -Parent
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
    $temporaryDirectory = Join-Path $outputParent ('.apollo-installer-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null
    $temporaryInstaller = Join-Path $temporaryDirectory 'Apollo-Setup-x64.exe'

    if (Test-Path -LiteralPath $outputPath) {
        Remove-Item -LiteralPath $outputPath -Force
    }

    try {
        $compilerArguments = @(
            '/V2',
            "/DVERSION=$($release.Version)",
            "/DPROJECT_ROOT=$projectRoot",
            "/DAPOLLO_EXE=$executablePath",
            "/DOUTPUT_FILE=$temporaryInstaller",
            $nsiPath
        )

        $LASTEXITCODE = 0
        & $compilerPath @compilerArguments
        if ($LASTEXITCODE -ne 0) {
            throw "NSIS compiler failed with exit code $LASTEXITCODE."
        }

        if (-not (Test-Path -LiteralPath $temporaryInstaller -PathType Leaf)) {
            $unexpected = @(Get-ChildItem -LiteralPath $temporaryDirectory -Filter '*.exe' -File -ErrorAction SilentlyContinue)
            if ($unexpected.Count -gt 0) {
                $names = ($unexpected | ForEach-Object { $_.Name } | Sort-Object) -join ', '
                throw "NSIS produced unexpected installer filename(s): $names. Expected Apollo-Setup-x64.exe."
            }
            throw "NSIS compiler did not produce the expected installer: $temporaryInstaller"
        }

        Move-Item -LiteralPath $temporaryInstaller -Destination $outputPath
    }
    finally {
        if (Test-Path -LiteralPath $temporaryDirectory) {
            Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
        }
    }

    Write-Host "Created: $outputPath"
}

if ($MyInvocation.InvocationName -ne '.') {
    Invoke-BuildWindowsInstallerCli -Arguments $args
}
