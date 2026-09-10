#requires -Version 7.0

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot 'WindowsRelease.Common.ps1')
. (Join-Path $PSScriptRoot 'stage-media-tools.ps1')
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

function Write-MediaToolsInstallerInclude {
    param([string]$MediaToolsPath, $Manifest, [string]$OutputPath)
    $install = [Collections.Generic.List[string]]::new()
    $uninstall = [Collections.Generic.List[string]]::new()
    $directories = [Collections.Generic.HashSet[string]]::new()
    $install.Add('!macro InstallMediaTools')
    $uninstall.Add('!macro UninstallMediaTools')
    foreach ($relative in (Get-MediaToolPayloadPaths $Manifest)) {
        $source = Join-Path $MediaToolsPath $relative
        Assert-InstallerSourceFile -Path $source -Description 'Staged media-tool payload'
        # Manifest paths are restricted portable names; escape the caller-owned absolute root.
        if ($source.Contains("`n") -or $source.Contains("`r") -or $source.Contains('"')) { throw 'Unsupported NSIS source path.' }
        $source = $source.Replace('$', '$$')
        $windowsPath = $relative.Replace('/', '\')
        $directory = [IO.Path]::GetDirectoryName($windowsPath)
        $destination = if ($directory) { '$INSTDIR\' + $directory } else { '$INSTDIR' }
        $install.Add('    SetOutPath "' + $destination + '"')
        $install.Add('    File "/oname=' + [IO.Path]::GetFileName($windowsPath) + '" "' + $source + '"')
        $uninstall.Add('    Delete "$INSTDIR\' + $windowsPath + '"')
        while ($directory) {
            [void]$directories.Add($directory)
            $directory = [IO.Path]::GetDirectoryName($directory)
        }
    }
    foreach ($directory in ($directories | Sort-Object Length -Descending)) {
        $uninstall.Add('    RMDir "$INSTDIR\' + $directory + '"')
    }
    $install.Add('    SetOutPath "$INSTDIR"')
    $install.Add('!macroend')
    $uninstall.Add('!macroend')
    [IO.File]::WriteAllLines($OutputPath, @($install) + @($uninstall), [Text.UTF8Encoding]::new($false))
}

function Invoke-BuildWindowsInstallerCli {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Arguments,
        # Internal release orchestration supplies the already validated shared pair.
        [string]$MediaToolsPath
    )

    $cli = ConvertFrom-CliArguments -Arguments $Arguments -OptionSpecs @{
        project = @{ Aliases = @('-p', '-ProjectRoot'); Default = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
        executable = @{ Aliases = @('-e', '-Executable') }
        makensis = @{ Aliases = @('-m', '-Makensis') }
        output = @{ Aliases = @('-o', '-Output') }
        'media-cache' = @{}
        offline = @{ Kind = 'Switch' }
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
  --media-cache <path>          Pinned archive cache. Defaults to .scratch/media-tools-cache.
  --offline                    Require a valid cached media archive.

Relative executable, compiler, and output paths resolve from the project root.
The repository-owned assets/apollo.ico file is required and validated before NSIS runs.
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
    $iconPath = [System.IO.Path]::GetFullPath((Join-Path $projectRoot 'assets/apollo.ico'))
    $nsiPath = Join-Path $projectRoot 'installer/windows/apollo.nsi'
    Assert-InstallerSourceFile -Path $licensePath -Description 'Project license'
    Assert-InstallerSourceFile -Path $noticesPath -Description 'Third-party notices'
    Assert-InstallerSourceFile -Path $iconPath -Description 'Apollo application icon'
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

    try {
        $manifestPath = Join-Path $projectRoot 'distribution/media-tools.json'
        $manifest = Read-MediaToolsManifest $manifestPath
        if ([string]::IsNullOrEmpty($MediaToolsPath)) {
            $cacheValue = if ($null -eq $cli['media-cache']) { '.scratch/media-tools-cache' } else { [string]$cli['media-cache'] }
            $cachePath = Resolve-ReleasePath -ProjectRoot $projectRoot -Path $cacheValue -OptionName '--media-cache'
            $MediaToolsPath = Publish-MediaTools -ManifestPath $manifestPath -CachePath $cachePath -OutputPath (Join-Path $temporaryDirectory 'media-tools') -Offline:$cli['offline']
        }
        $includePath = Join-Path $temporaryDirectory 'media-tools.nsh'
        Write-MediaToolsInstallerInclude -MediaToolsPath $MediaToolsPath -Manifest $manifest -OutputPath $includePath
        $compilerArguments = @(
            '/V2',
            "/DVERSION=$($release.Version)",
            "/DPROJECT_ROOT=$projectRoot",
            "/DAPOLLO_EXE=$executablePath",
            "/DAPP_ICON=$iconPath",
            "/DOUTPUT_FILE=$temporaryInstaller",
            "/DMEDIA_TOOLS_INCLUDE=$includePath",
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

        if (Test-Path -LiteralPath $outputPath) {
            [IO.File]::Replace($temporaryInstaller, $outputPath, $null)
        }
        else { [IO.File]::Move($temporaryInstaller, $outputPath) }
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
