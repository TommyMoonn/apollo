#requires -Version 7.0

function New-ReleaseMediaFixture {
    param([string]$Root, [switch]$ExtraLicense)
    $manifest = Get-Content (Join-Path $PSScriptRoot '../../distribution/media-tools.json') -Raw | ConvertFrom-Json -AsHashtable
    if ($ExtraLicense) {
        $manifest.redistribution += @{ source = 'extra/NOTICE.txt'; destination = 'licenses/ffmpeg/extra/NOTICE.txt'; purpose = 'test extra license' }
    }
    $entries = @{}
    foreach ($tool in @('ffmpeg', 'ffprobe')) { $entries[$manifest.executables[$tool]] = "$tool fixture" }
    foreach ($item in $manifest.redistribution) { $entries[$item.source] = "redistribution fixture: $($item.source)" }
    $entries['unrelated/ffplay.exe'] = 'excluded'
    $cache = Join-Path $Root '.scratch/media-tools-cache'
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    $archive = Join-Path $cache $manifest.archive.filename
    New-TestZip -Path $archive -Entries $entries | Out-Null
    $manifest.archive.sha256 = (Get-FileHash -LiteralPath $archive).Hash.ToLowerInvariant()
    New-Item -ItemType Directory -Path (Join-Path $Root 'distribution') -Force | Out-Null
    $manifest | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $Root 'distribution/media-tools.json')
}

function Invoke-ReleaseScriptFixture {
    param([string]$ScriptPath, [object[]]$Arguments = @())
    # Substitute only native media execution; manifest, ZIP, checksums and packaging are real.
    $captured = @(& {
        . $ScriptPath
        function Invoke-MediaTool {
            param([string]$Path, [string[]]$Arguments)
            $name = [IO.Path]::GetFileNameWithoutExtension($Path)
            Assert-Equal "$name fixture" ([IO.File]::ReadAllText($Path)) 'Validate the staged fixture executable.'
            if ($Arguments -contains '-version') { return "$name version 7.0.2-essentials_build-www.gyan.dev Copyright" }
            return (@('libx264','libx265','h264_nvenc','hevc_nvenc','aac','flac','png','mjpeg') | ForEach-Object { " V....D $_ fixture" }) -join "`n"
        }
        if ([IO.Path]::GetFileName($ScriptPath) -eq 'stage-windows-release.ps1') {
            Invoke-StageWindowsReleaseCli -Arguments (@($Arguments) + '--offline')
        }
        else { Invoke-BuildWindowsInstallerCli -Arguments (@($Arguments) + '--offline') }
    } 6>&1)
    return ($captured | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
}
