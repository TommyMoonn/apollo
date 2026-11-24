#requires -Version 7.0
# Runtime acceptance implementation; use scripts/smoke-windows-installer.ps1.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-SmokeProcess {
    param([string]$Executable, [string]$RawArguments, [string]$Scratch, [switch]$Discovery,
        [ValidateSet('cpu','nvenc','unavailable')][string]$Acceptance = 'cpu', [int]$TimeoutMilliseconds = 180000)
    $info = [Diagnostics.ProcessStartInfo]::new($Executable)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.WorkingDirectory = $Scratch
    $info.Arguments = $RawArguments
    $info.Environment['PATH'] = ''
    if ($Discovery) {
        $info.Environment['APOLLO_DISCOVERY_TEST_MODE'] = 'bundle'
        [void]$info.Environment.Remove('APOLLO_TEST_REQUIRE_NVENC')
        $info.Environment['APOLLO_RELEASE_ACCEPTANCE'] = $Acceptance
        $info.Environment['APOLLO_RELEASE_OUTPUT_DIRECTORY'] = Join-Path $Scratch ([Guid]::NewGuid().ToString('N'))
        if ($Acceptance -eq 'unavailable') { $info.Environment['CUDA_VISIBLE_DEVICES'] = '-1' }
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        [void]$process.Start()
        if ($Discovery) {
            $stdout = $process.StandardOutput.ReadToEndAsync()
            $stderr = $process.StandardError.ReadToEndAsync()
        }
        if (-not $process.WaitForExit($TimeoutMilliseconds)) {
            $process.Kill($true)
            if (-not $process.WaitForExit(10000)) { throw "Smoke process could not be stopped: $Executable" }
            throw [TimeoutException]::new("Smoke process timed out: $Executable")
        }
        if ($Discovery -and -not [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]@($stdout,$stderr), 10000)) {
            throw [TimeoutException]::new("Smoke process diagnostics did not close: $Executable")
        }
        $diagnostic = if ($Discovery) { $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult() } else { '' }
        if ($process.ExitCode -ne 0) { throw "Smoke process failed ($($process.ExitCode)): $Executable`n$diagnostic" }
        if ($Discovery) {
            if (-not $diagnostic.Contains('PASS discovery: bundle')) { throw "Discovery harness did not run: $diagnostic" }
            if (-not $diagnostic.Contains("PASS release acceptance: $Acceptance")) { throw "Operation acceptance did not run: $diagnostic" }
            Write-Host $diagnostic.Trim()
        }
    }
    finally { $process.Dispose() }
}

function Get-SmokePredecessor {
    param([object[]]$Releases, [string]$Version, [switch]$UpgradeOnly)
    $eligible = @($Releases | Where-Object {
        -not $_.draft -and -not $_.prerelease -and $_.tag_name -cmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' -and
        $(try { [version]$_.tag_name.Substring(1) -le [version]$Version } catch { $false }) -and
        (-not $UpgradeOnly -or $_.tag_name -cne "v$Version")
    } | Sort-Object { [version]$_.tag_name.Substring(1) } -Descending)
    if ($eligible.Count -eq 0) { throw 'Upgrade acceptance blocked: no eligible published Apollo predecessor.' }
    $selected = $eligible[0]
    if (@($eligible | Where-Object tag_name -CEQ $selected.tag_name).Count -ne 1) { throw 'Ambiguous predecessor release.' }
    foreach ($name in @('Apollo-Setup-x64.exe','SHA256SUMS.txt')) {
        $assets = @($selected.assets | Where-Object name -CEQ $name)
        if ($assets.Count -ne 1 -or $assets[0].state -cne 'uploaded' -or $assets[0].size -le 0) {
            throw "Upgrade acceptance blocked: $($selected.tag_name) has no verified $name asset."
        }
    }
    return $selected
}

function Assert-SmokePredecessorChecksum {
    param([string]$Directory, $Release)
    foreach ($name in @('Apollo-Setup-x64.exe','SHA256SUMS.txt')) {
        $asset = @($Release.assets | Where-Object name -CEQ $name)[0]
        $record = Get-ReleaseFileRecord (Join-Path $Directory $name)
        if ($record.size -ne $asset.size) { throw "Predecessor asset size mismatch: $name" }
    }
    $checksums = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
    foreach ($line in [IO.File]::ReadAllLines((Join-Path $Directory 'SHA256SUMS.txt'))) {
        if ($line -cnotmatch '\A([0-9a-fA-F]{64})  (Apollo-Setup-x64\.exe|Apollo-windows-x64\.zip)\z' -or
            -not $checksums.TryAdd($Matches[2], $Matches[1].ToLowerInvariant())) { throw 'Invalid or duplicate predecessor checksum.' }
    }
    $installer = Join-Path $Directory 'Apollo-Setup-x64.exe'
    if (-not $checksums.ContainsKey('Apollo-Setup-x64.exe') -or
        (Get-ReleaseFileRecord $installer).sha256 -cne $checksums['Apollo-Setup-x64.exe']) { throw 'Predecessor installer checksum mismatch.' }
    return $installer
}

function Get-SmokePredecessorInstaller {
    param([string]$Scratch, [string]$Version, [switch]$UpgradeOnly)
    # Do not accept an arbitrary executable or silently fall back to an older release.
    $repo = 'TommyMoonn/apollo'
    $raw = & gh api --paginate --slurp "repos/$repo/releases?per_page=100"
    if ($LASTEXITCODE -ne 0) { throw 'Upgrade acceptance blocked: could not enumerate official releases.' }
    $pages = ($raw -join "`n") | ConvertFrom-Json -NoEnumerate
    $releases = @(foreach ($page in $pages) { foreach ($release in $page) { $release } })
    $selected = Get-SmokePredecessor -Releases $releases -Version $Version -UpgradeOnly:$UpgradeOnly
    $directory = Join-Path $Scratch 'predecessor'
    [void][IO.Directory]::CreateDirectory($directory)
    & gh release download $selected.tag_name --repo $repo --pattern 'Apollo-Setup-x64.exe' --pattern 'SHA256SUMS.txt' --dir $directory | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Upgrade acceptance blocked: official predecessor download failed.' }
    $installer = Assert-SmokePredecessorChecksum $directory $selected
    Write-Host "Verified official predecessor $($selected.tag_name) (release $($selected.id), installer SHA256 $((Get-ReleaseFileRecord $installer).sha256))"
    return @{ Path = $installer; Version = $selected.tag_name.Substring(1) }
}

function Get-SmokeHarness {
    param([string]$ProjectRoot, [string]$DiscoveryHarness)
    if ($DiscoveryHarness) {
        $path = Resolve-ReleasePath $ProjectRoot $DiscoveryHarness '--discovery-harness'
        $null = Get-ReleaseFileRecord $path
        return $path
    }
    Push-Location $ProjectRoot
    try {
        $artifacts = @(& cargo test --locked --test bundled_media_tools --no-run --message-format=json |
            ForEach-Object { $_ | ConvertFrom-Json } |
            Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'bundled_media_tools' -and $_.executable })
        if ($LASTEXITCODE -ne 0 -or $artifacts.Count -ne 1) { throw 'Discovery harness build failed.' }
        return $artifacts[0].executable
    } finally { Pop-Location }
}

function Invoke-SmokeMedia {
    param([string]$Directory, [string]$Harness, [string]$Scratch, [switch]$RequireNvenc)
    $probe = Join-Path $Directory 'discovery-test.exe'
    if (Test-Path -LiteralPath $probe) { throw "Unexpected discovery harness destination: $probe" }
    Copy-Item -LiteralPath $Harness -Destination $probe
    try {
        foreach ($mode in @('cpu','unavailable')) {
            Invoke-SmokeProcess $probe '--ignored --exact discovery_subprocess --nocapture' -Scratch $Scratch -Discovery -Acceptance $mode
        }
        if ($RequireNvenc) { Invoke-SmokeProcess $probe '--ignored --exact discovery_subprocess --nocapture' -Scratch $Scratch -Discovery -Acceptance nvenc }
        else { Write-Host 'SKIP NVIDIA success: --require-nvenc not selected.' }
    } finally { Remove-Item -LiteralPath $probe -Force }
}

function Assert-SmokePayload {
    param([string]$Directory, $Payload, [string[]]$ExtraFiles = @())
    Assert-TransferUnlinked $Directory
    $actual = @(Get-ChildItem -LiteralPath $Directory -Recurse -Force | ForEach-Object {
        if ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Unexpected linked installed path: $($_.FullName)" }
        if (-not $_.PSIsContainer) { [IO.Path]::GetRelativePath($Directory, $_.FullName).Replace('\','/') }
    })
    $expected = @($Payload.Keys) + $ExtraFiles
    if (@(Compare-Object ($expected | Sort-Object) ($actual | Sort-Object) -CaseSensitive).Count -ne 0) { throw "Unexpected payload in $Directory" }
    foreach ($name in $Payload.Keys) {
        Assert-ReleaseRecord $Payload[$name] (Get-ReleaseFileRecord (Join-Path $Directory $name)) $name
    }
}

function Expand-SmokePortable {
    param([string]$ArchivePath, [string]$Directory, $Payload)
    [void][IO.Directory]::CreateDirectory($Directory)
    $archive = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        if ($archive.Entries.Count -ne $Payload.Count) { throw 'Portable payload entry count changed.' }
        foreach ($name in $Payload.Keys) {
            $entry = $archive.GetEntry($name)
            if ($null -eq $entry -or $entry.Length -ne $Payload[$name].size) { throw "Portable entry missing or changed: $name" }
            # Destination paths come from the already verified payload contract.
            $destination = Join-Path $Directory $name
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destination, $false)
        }
    } finally { $archive.Dispose() }
    Assert-SmokePayload $Directory $Payload
}

function Assert-SmokeCleanState {
    param([string[]]$RegistryPaths, [string[]]$Shortcuts, [string]$DefaultInstall)
    foreach ($path in ($RegistryPaths + $Shortcuts + @($DefaultInstall))) {
        if (Test-Path -LiteralPath $path) { throw "Release smoke requires a clean Apollo installation state; existing path: $path" }
    }
    if (Get-Process -Name Apollo -ErrorAction SilentlyContinue) { throw 'Apollo is running; use an isolated Windows user or VM.' }
}

function Assert-SmokeRegistration {
    param([string]$Directory, [string]$Version, [string[]]$RegistryPaths, [string[]]$Shortcuts)
    $exe = Join-Path $Directory 'Apollo.exe'
    $uninstaller = Join-Path $Directory 'Uninstall.exe'
    $null = Get-ReleaseFileRecord $uninstaller
    Assert-ApolloResourceVersion $exe $Version
    $registration = Get-ItemProperty -LiteralPath $RegistryPaths[0]
    if ($registration.DisplayName -cne 'Apollo' -or $registration.DisplayVersion -cne $Version -or
        $registration.InstallLocation -ine $Directory -or $registration.DisplayIcon -ine $exe -or
        $registration.UninstallString -ine ('"' + $uninstaller + '"') -or
        (Get-Item -LiteralPath $RegistryPaths[1]).GetValue('') -ine $exe) { throw 'Incorrect Apollo per-user registration.' }
    if (-not (Test-Path -LiteralPath $Shortcuts[0] -PathType Leaf)) { throw 'Start menu shortcut missing.' }
    $shell = New-Object -ComObject WScript.Shell
    foreach ($path in $Shortcuts) {
        if ((Test-Path -LiteralPath $path) -and $shell.CreateShortcut($path).TargetPath -ine $exe) { throw "Wrong shortcut target: $path" }
    }
}

function Assert-SmokeSentinels {
    param([string]$Directory, $Sentinels)
    foreach ($name in $Sentinels.Keys) { Assert-ReleaseRecord $Sentinels[$name] (Get-ReleaseFileRecord (Join-Path $Directory $name)) $name }
}

function New-SmokeSentinels {
    param([string]$Directory)
    $records = [ordered]@{}
    foreach ($name in @('keep.txt','licenses/ffmpeg/keep.txt')) {
        $path = Join-Path $Directory $name
        Assert-TransferUnlinked $path
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))
        if (Test-Path -LiteralPath $path) { throw "Sentinel destination already exists: $path" }
        [IO.File]::WriteAllText($path, [guid]::NewGuid().ToString())
        $records[$name] = Get-ReleaseFileRecord $path
    }
    return $records
}

function Invoke-SmokeUninstall {
    param([string]$Directory, [string]$Scratch, $Payload, $Sentinels, [string[]]$RegistryPaths, [string[]]$Shortcuts)
    # Run a separate copy so the installer-owned Uninstall.exe is not locked by Windows.
    $copy = Join-Path $Scratch ('uninstall-' + [guid]::NewGuid().ToString('N') + '.exe')
    Copy-Item -LiteralPath (Join-Path $Directory 'Uninstall.exe') -Destination $copy
    try { Invoke-SmokeProcess $copy ("/S _?=$Directory") -Scratch $Scratch }
    finally { Remove-Item -LiteralPath $copy -Force }
    foreach ($name in @($Payload.Keys) + @('Uninstall.exe')) {
        if (Test-Path -LiteralPath (Join-Path $Directory $name)) { throw "Uninstall retained owned file: $name" }
    }
    foreach ($path in ($RegistryPaths + $Shortcuts)) {
        if (Test-Path -LiteralPath $path) { throw "Uninstall retained registration or shortcut: $path" }
    }
    Assert-SmokeSentinels $Directory $Sentinels
}

function Remove-OwnedSmokeState {
    param([string[]]$Directories, [string[]]$RegistryPaths, [string[]]$Shortcuts)
    # The clean-state check ran before installation. Still check ownership before
    # deleting a key or shortcut, in case unrelated state appeared during the run.
    foreach ($path in $RegistryPaths) {
        if (-not (Test-Path -LiteralPath $path)) { continue }
        $key = Get-Item -LiteralPath $path
        $owned = if ($path -eq $RegistryPaths[0]) { $Directories -contains $key.GetValue('InstallLocation') } else {
            @($Directories | ForEach-Object { Join-Path $_ 'Apollo.exe' }) -contains $key.GetValue('')
        }
        if (-not $owned -or $key.SubKeyCount -ne 0) { throw "Refusing cleanup of unowned registration: $path" }
        Remove-Item -LiteralPath $path -Force
    }
    foreach ($path in $Shortcuts) {
        if (-not (Test-Path -LiteralPath $path)) { continue }
        $shell = New-Object -ComObject WScript.Shell
        if (@($Directories | ForEach-Object { Join-Path $_ 'Apollo.exe' }) -notcontains $shell.CreateShortcut($path).TargetPath) { throw "Refusing cleanup of unowned shortcut: $path" }
        Remove-Item -LiteralPath $path -Force
    }
}

function Invoke-WindowsReleaseSmoke {
    param([string]$ProjectRoot, [string]$ReleaseDirectory, [string]$ExpectedVersion, [string]$ExpectedSha,
        [switch]$Local, [string]$DiscoveryHarness, [switch]$RequireNvenc, [switch]$UpgradeOnly, [string]$MediaDirectory)
    if (-not $IsWindows -or -not [Environment]::Is64BitProcess) { throw 'Windows x64 PowerShell is required.' }
    $scratchRoot = Join-Path $ProjectRoot '.scratch'
    Assert-TransferUnlinked $scratchRoot
    $scratch = Join-Path $scratchRoot ('release-smoke-' + [guid]::NewGuid().ToString('N'))
    $registry = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Apollo', 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\Apollo.exe')
    $guardRegistry = $registry + @(
        'HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Apollo',
        'HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\Apollo.exe',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Apollo',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Apollo',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\App Paths\Apollo.exe',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\Apollo.exe'
    )
    $defaultInstall = Join-Path $env:LOCALAPPDATA 'Programs/Apollo'
    $shortcuts = @((Join-Path ([Environment]::GetFolderPath('Programs')) 'Apollo.lnk'), (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Apollo.lnk'))
    $installed = Join-Path $scratch 'installed'
    $upgraded = Join-Path $scratch 'upgrade'
    $portable = Join-Path $scratch 'portable'
    $mutex = [Threading.Mutex]::new($false, 'Local\ApolloReleaseSmoke')
    $locked = $false
    $clean = $false
    $scratchCreated = $false
    try {
        try { $locked = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $locked = $true }
        if (-not $locked) { throw 'Another Apollo acceptance run is active.' }
        if (-not $MediaDirectory) {
            Assert-SmokeCleanState $guardRegistry $shortcuts $defaultInstall
            Assert-WindowsReleaseArtifacts -ProjectRoot $ProjectRoot -ArtifactsDirectory $ReleaseDirectory -ExpectedVersion $ExpectedVersion -ExpectedSha $ExpectedSha -Local:$Local
            $provenance = Read-ReleaseProvenance (Join-Path $ReleaseDirectory 'release-provenance.json')
        }
        if (Test-Path -LiteralPath $scratch) { throw 'Smoke scratch path already exists.' }
        [void][IO.Directory]::CreateDirectory($scratch)
        $scratchCreated = $true
        Write-Host "Acceptance scratch: $scratch"
        $harness = Get-SmokeHarness $ProjectRoot $DiscoveryHarness
        if ($MediaDirectory) {
            [void][IO.Directory]::CreateDirectory($portable)
            foreach ($name in @('ffmpeg.exe','ffprobe.exe')) {
                $source = Join-Path $MediaDirectory $name
                $null = Get-ReleaseFileRecord $source
                Copy-Item -LiteralPath $source -Destination (Join-Path $portable $name)
            }
            Invoke-SmokeMedia $portable $harness $scratch -RequireNvenc:$RequireNvenc
            return
        }
        # The verifier has already checked every exact ZIP path and digest.
        Expand-SmokePortable (Join-Path $ReleaseDirectory 'Apollo-windows-x64.zip') $portable $provenance.payload
        Invoke-SmokeMedia $portable $harness $scratch -RequireNvenc:$RequireNvenc
        $installer = Join-Path $scratch 'Apollo-Setup-x64.exe'
        Copy-Item -LiteralPath (Join-Path $ReleaseDirectory 'Apollo-Setup-x64.exe') -Destination $installer
        Assert-ReleaseRecord $provenance.assets['Apollo-Setup-x64.exe'] (Get-ReleaseFileRecord $installer) 'Apollo-Setup-x64.exe'
        # Recheck clean state immediately before any installer mutation.
        Assert-SmokeCleanState $guardRegistry $shortcuts $defaultInstall
        $clean = $true
        Invoke-SmokeProcess $installer ("/S /D=$installed") -Scratch $scratch
        Assert-SmokeRegistration $installed $ExpectedVersion $registry $shortcuts
        if (Test-Path -LiteralPath $shortcuts[1]) { throw 'Silent clean install unexpectedly created a desktop shortcut.' }
        Assert-SmokePayload $installed $provenance.payload -ExtraFiles @('Uninstall.exe')
        Invoke-SmokeMedia $installed $harness $scratch -RequireNvenc:$RequireNvenc
        $sentinels = New-SmokeSentinels $installed
        Invoke-SmokeUninstall $installed $scratch $provenance.payload $sentinels $registry $shortcuts
        Write-Host 'PASS clean install, portable processing, uninstall and user-file preservation.'

        $predecessor = Get-SmokePredecessorInstaller $scratch $ExpectedVersion -UpgradeOnly:$UpgradeOnly
        Assert-SmokeCleanState $guardRegistry $shortcuts $defaultInstall
        Invoke-SmokeProcess $predecessor.Path ("/S /D=$upgraded") -Scratch $scratch
        Assert-SmokeRegistration $upgraded $predecessor.Version $registry $shortcuts
        $sentinels = New-SmokeSentinels $upgraded
        # Represent a user who enabled the optional desktop shortcut previously.
        $shell = New-Object -ComObject WScript.Shell
        $desktopShortcut = $shell.CreateShortcut($shortcuts[1])
        $desktopShortcut.TargetPath = Join-Path $upgraded 'Apollo.exe'
        $desktopShortcut.Save()
        # No /D override: the candidate must reuse the registered location.
        Invoke-SmokeProcess $installer '/S' -Scratch $scratch
        Assert-SmokeRegistration $upgraded $ExpectedVersion $registry $shortcuts
        if (-not (Test-Path -LiteralPath $shortcuts[1] -PathType Leaf)) { throw 'Upgrade removed the previously enabled desktop shortcut.' }
        Assert-SmokeSentinels $upgraded $sentinels
        Assert-SmokePayload $upgraded $provenance.payload -ExtraFiles (@('Uninstall.exe') + @($sentinels.Keys))
        Invoke-SmokeUninstall $upgraded $scratch $provenance.payload $sentinels $registry $shortcuts
        $kind = if ($predecessor.Version -ceq $ExpectedVersion) { 'same-version reinstall' } else { 'cross-version upgrade' }
        Write-Host "PASS $kind ($($predecessor.Version) -> $ExpectedVersion): location, registration, payload and user-file preservation."
    } finally {
        try {
            if ($clean) { Remove-OwnedSmokeState @($installed,$upgraded) $registry $shortcuts }
            if ($scratchCreated -and (Test-Path -LiteralPath $scratch)) {
                Assert-TransferUnlinked $scratch
                Remove-Item -LiteralPath $scratch -Recurse -Force
            }
        } finally {
            if ($locked) { $mutex.ReleaseMutex() }
            $mutex.Dispose()
        }
    }
}
