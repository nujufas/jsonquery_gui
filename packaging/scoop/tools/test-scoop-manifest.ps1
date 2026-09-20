<#
.SYNOPSIS
    Tests the Scoop manifest end to end on this Windows VM.

.DESCRIPTION
    Installs Scoop if it is missing, then runs Scoop's own checks (checkver, checkhashes,
    formatjson) on the manifest, installs the app from it, verifies what landed on disk
    (files, `jsonquery-gui` shim, Start-menu shortcut), optionally launches it, uninstalls
    and confirms nothing is left behind.

    Scoop's INSTALLER refuses to run from an elevated shell unless given -RunAsAdmin
    (Scoop itself does not mind). A normal user is the realistic case, but the Docker
    Windows VM has UAC switched off, so everything in it is elevated; the script detects
    that and passes -RunAsAdmin to the installer. Either way, double-click
    test-scoop-manifest.cmd.

    A transcript and latest-scoop.txt / latest-result.txt are written to .\logs next to this
    script (<WINVM_DIR>/shared/logs on the Linux host, see windows-vm.sh, which stages this
    script and the manifest into the share).

.PARAMETER Manifest
    The manifest to test. Default: scoop\jsonquery-gui.json next to this script.

.PARAMETER Bucket
    Test the PUBLISHED bucket instead of the local file: a git URL such as
    https://github.com/nujufas/scoop-jsonquery-gui. Adds the bucket (installing git first
    if needed), installs <bucket>/<app> from it and removes the bucket again afterwards.

.PARAMETER KeepInstalled
    Leave the app installed at the end instead of uninstalling it.

.PARAMETER Launch
    Also start the installed app and report whether its process stays alive.

.PARAMETER NoPause
    Do not wait for Enter at the end.
#>
[CmdletBinding()]
param(
    [string]$Manifest,
    [string]$Bucket,
    [switch]$KeepInstalled,
    [switch]$Launch,
    [switch]$NoPause
)

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$root = $PSScriptRoot
$logDir = Join-Path $root 'logs'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$logFile = Join-Path $logDir ('scoop-test-{0}.log' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
Start-Transcript -Path $logFile -Force | Out-Null

$results = New-Object System.Collections.ArrayList

function Add-Result([string]$Step, [string]$Status, [string]$Detail = '') {
    [void]$script:results.Add([pscustomobject]@{ Step = $Step; Status = $Status; Detail = $Detail })
    $color = 'Red'
    if ($Status -eq 'PASS') { $color = 'Green' }
    elseif ($Status -eq 'WARN') { $color = 'Yellow' }
    elseif ($Status -eq 'SKIP') { $color = 'DarkGray' }
    Write-Host ('  [{0}] {1}  {2}' -f $Status, $Step, $Detail) -ForegroundColor $color
}

function Write-Step([string]$Text) {
    Write-Host ''
    Write-Host "== $Text" -ForegroundColor Cyan
}

function Invoke-Scoop([string[]]$ScoopArgs) {
    # Runs scoop with its normal console output; returns the exit code (not always set by Scoop).
    Write-Host ('> scoop ' + ($ScoopArgs -join ' ')) -ForegroundColor DarkGray
    $global:LASTEXITCODE = 0
    & scoop @ScoopArgs | Out-Host
    return [int]$LASTEXITCODE
}

function Get-ScriptOutput([scriptblock]$Block) {
    # Runs a Scoop bin\*.ps1 and returns everything it printed (Write-Host included) as text.
    $text = (& $Block *>&1 | ForEach-Object { "$_" } | Out-String).Trim()
    Write-Host $text
    return $text
}

try {
    $os = Get-CimInstance Win32_OperatingSystem
    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    Write-Host ''
    Write-Host 'Scoop manifest test' -ForegroundColor Cyan
    Write-Host ('  log : {0}' -f $logFile)
    Write-Host ('  OS  : {0} build {1}' -f $os.Caption, $os.BuildNumber)
    if ($isAdmin) {
        Add-Result 'session' 'PASS' "elevated (UAC is off in this VM), so Scoop's installer gets -RunAsAdmin"
    } else {
        Add-Result 'session' 'PASS' 'normal (non-elevated) user'
    }

    # 1. the manifest under test
    Write-Step '1. Manifest'
    if (-not $Manifest) { $Manifest = Join-Path $root 'scoop\jsonquery-gui.json' }
    if (-not (Test-Path $Manifest)) {
        Add-Result 'manifest file' 'FAIL' "not found: $Manifest (pass -Manifest, or run: windows-vm.sh stage scoop)"
        return
    }
    $m = Get-Content $Manifest -Raw | ConvertFrom-Json
    $name = [IO.Path]::GetFileNameWithoutExtension($Manifest)
    $version = $m.version
    $exeName = $null
    $alias = $null
    if ($m.bin) {
        $first = @($m.bin)[0]
        if ($first -is [string]) { $exeName = $first; $alias = [IO.Path]::GetFileNameWithoutExtension($first) }
        else { $exeName = @($first)[0]; $alias = @($first)[1] }
    }
    $shortcutName = $null
    if ($m.shortcuts) { $shortcutName = @(@($m.shortcuts)[0])[1] }
    Add-Result 'manifest file' 'PASS' ('{0} {1} ({2})' -f $name, $version, $Manifest)

    # a local copy avoids network-path quirks, and Scoop's bin\*.ps1 want a directory
    $work = Join-Path $env:TEMP 'scoop-test'
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    Copy-Item $Manifest (Join-Path $work "$name.json") -Force
    $checkDir = $work

    # 2. Scoop itself
    Write-Step '2. Scoop'
    $scoopRoot = $env:SCOOP
    if (-not $scoopRoot) { $scoopRoot = Join-Path $env:USERPROFILE 'scoop' }
    if (-not (Get-Command scoop -ErrorAction SilentlyContinue) -and (Test-Path (Join-Path $scoopRoot 'shims'))) {
        $env:Path = (Join-Path $scoopRoot 'shims') + ';' + $env:Path
    }
    if (-not (Get-Command scoop -ErrorAction SilentlyContinue)) {
        Write-Host '  Scoop is not installed - installing it from https://get.scoop.sh'
        try {
            if ((Get-ExecutionPolicy) -notin @('Bypass', 'Unrestricted', 'RemoteSigned')) {
                try { Set-ExecutionPolicy RemoteSigned -Scope CurrentUser -Force -ErrorAction Stop } catch { Write-Host ('  execution policy: ' + $_.Exception.Message) }
            }
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            $installer = Invoke-RestMethod -Uri 'https://get.scoop.sh'
            if ($isAdmin) { Invoke-Expression "& {$installer} -RunAsAdmin" }
            else { Invoke-Expression $installer }
        } catch {
            Add-Result 'install Scoop' 'FAIL' $_.Exception.Message
            return
        }
        $env:Path = (Join-Path $scoopRoot 'shims') + ';' + $env:Path
    }
    if (Get-Command scoop -ErrorAction SilentlyContinue) {
        $ver = (& scoop --version *>&1 | ForEach-Object { "$_" } | Where-Object { $_ -match '\S' } | Select-Object -First 2) -join ' | '
        Add-Result 'Scoop' 'PASS' $ver
    } else {
        Add-Result 'Scoop' 'FAIL' 'scoop is not on PATH after installing it'
        return
    }
    $scoopHome = ((& scoop prefix scoop 2>$null) | Select-Object -Last 1)
    New-Item -ItemType Directory -Force -Path (Join-Path $scoopRoot 'cache') | Out-Null # checkhashes.ps1 trips over a missing cache folder

    # 3. the published bucket, when asked for
    $target = Join-Path $work "$name.json"
    $bucketName = 'jsonquery-gui'
    if ($Bucket) {
        Write-Step '3. Bucket'
        if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
            [void](Invoke-Scoop @('install', 'git'))
            $env:Path = (Join-Path $scoopRoot 'shims') + ';' + $env:Path
        }
        [void](Invoke-Scoop @('bucket', 'rm', $bucketName))
        [void](Invoke-Scoop @('bucket', 'add', $bucketName, $Bucket))
        $bucketDir = Join-Path $scoopRoot "buckets\$bucketName"
        $bucketManifest = @(Get-ChildItem $bucketDir -Recurse -Filter "$name.json" -ErrorAction SilentlyContinue)[0]
        if ($bucketManifest) {
            Add-Result 'bucket' 'PASS' ('{0} -> {1}' -f $Bucket, $bucketManifest.FullName)
            $checkDir = $bucketManifest.DirectoryName
            $target = "$bucketName/$name"
        } else {
            Add-Result 'bucket' 'FAIL' "no $name.json found in $bucketDir after 'scoop bucket add'"
            return
        }
    }

    # 4. Scoop's own maintenance checks, the same ones the bucket's CI and Excavator run
    Write-Step '4. Scoop checks (checkver / checkhashes / formatjson)'
    if ($scoopHome -and (Test-Path (Join-Path $scoopHome 'bin\checkver.ps1'))) {
        $bin = Join-Path $scoopHome 'bin'
        $out = Get-ScriptOutput { & (Join-Path $bin 'checkver.ps1') -App $name -Dir $checkDir }
        if ($out -match [regex]::Escape($name) + ':\s*(\S+)') {
            $latest = $Matches[1]
            if ($out -match 'scoop version is') { Add-Result 'checkver' 'WARN' "latest release is $latest but the manifest says $version (autoupdate would bump it)" }
            else { Add-Result 'checkver' 'PASS' "latest release $latest matches the manifest" }
        } else {
            Add-Result 'checkver' 'FAIL' 'no version detected from the homepage / releases'
        }
        $out = Get-ScriptOutput { & (Join-Path $bin 'checkhashes.ps1') -App $name -Dir $checkDir }
        if ($out -match [regex]::Escape($name) + ':\s*OK') { Add-Result 'checkhashes' 'PASS' 'the download matches the manifest hash' }
        elseif ($out -match 'Mismatch') { Add-Result 'checkhashes' 'FAIL' 'hash mismatch (see the log)' }
        else { Add-Result 'checkhashes' 'WARN' 'unexpected output (see the log)' }
        if (-not $Bucket) {
            $before = (Get-FileHash (Join-Path $work "$name.json") -Algorithm SHA256).Hash
            [void](Get-ScriptOutput { & (Join-Path $bin 'formatjson.ps1') -App $name -Dir $work })
            $after = (Get-FileHash (Join-Path $work "$name.json") -Algorithm SHA256).Hash
            if ($before -eq $after) { Add-Result 'formatjson' 'PASS' 'manifest is already formatted the way Scoop writes it' }
            else {
                Copy-Item (Join-Path $work "$name.json") (Join-Path $logDir "$name.formatted.json") -Force
                Add-Result 'formatjson' 'WARN' "Scoop would reformat the file - see logs\$name.formatted.json"
                Copy-Item $Manifest (Join-Path $work "$name.json") -Force
            }
        }
    } else {
        Add-Result 'Scoop checks' 'WARN' "Scoop's bin\ scripts not found (scoop prefix scoop = '$scoopHome')"
    }

    # 5. install (removing a previous install first)
    Write-Step '5. scoop install'
    if (Test-Path (Join-Path $scoopRoot "apps\$name")) {
        Write-Host '  found a previous install - removing it first'
        [void](Invoke-Scoop @('uninstall', $name))
    }
    [void](Invoke-Scoop @('cache', 'rm', $name))
    $code = Invoke-Scoop @('install', $target)
    $appDir = Join-Path $scoopRoot "apps\$name"
    $exe = Join-Path $appDir "current\$exeName"
    $installed = (Test-Path $exe)
    if ($installed) { Add-Result 'scoop install' 'PASS' "exit $code" }
    else { Add-Result 'scoop install' 'FAIL' "exit $code, and $exe does not exist" }
    [void](Invoke-Scoop @('list'))

    # 6. what actually landed on disk
    Write-Step '6. Installed files, shim and shortcut'
    $env:Path = (Join-Path $scoopRoot 'shims') + ';' + [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($installed) {
        $fi = Get-Item $exe
        Add-Result 'installed exe' 'PASS' ('{0} ({1:N1} MB)' -f $fi.FullName, ($fi.Length / 1MB))
        if (Test-Path (Join-Path $appDir "$version\$exeName")) { Add-Result 'version folder' 'PASS' "apps\$name\$version" }
        else { Add-Result 'version folder' 'FAIL' "apps\$name\$version is missing (manifest says $version)" }
        if ($alias) {
            $cmd = Get-Command $alias -ErrorAction SilentlyContinue
            if ($cmd) { Add-Result "command '$alias'" 'PASS' $cmd.Source }
            else { Add-Result "command '$alias'" 'FAIL' 'not found on PATH after install' }
        }
        if ($shortcutName) {
            $lnk = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Scoop Apps\$shortcutName.lnk"
            if (Test-Path $lnk) {
                $target = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk).TargetPath
                Add-Result 'Start-menu shortcut' 'PASS' "$shortcutName -> $target"
            } else {
                Add-Result 'Start-menu shortcut' 'FAIL' "missing: $lnk"
            }
        }
        $sig = Get-AuthenticodeSignature $exe
        if ($sig.Status -eq 'Valid') { Add-Result 'Authenticode' 'PASS' "signed by $($sig.SignerCertificate.Subject)" }
        else { Add-Result 'Authenticode' 'WARN' "$($sig.Status) - unsigned exe, so expect SmartScreen/reputation warnings" }
    }

    # 7. optional: does the GUI start? (this VM has no GPU, so a failure here is not the package's fault)
    if ($Launch -and $installed) {
        Write-Step '7. Launch'
        $p = Start-Process -FilePath $exe -PassThru
        Start-Sleep -Seconds 8
        if ($p.HasExited) {
            Add-Result 'launch' 'WARN' "exited with code $($p.ExitCode) within 8 s (no GPU in this VM, so OpenGL may be unavailable)"
        } else {
            Add-Result 'launch' 'PASS' 'process still running after 8 s (closing it)'
            Stop-Process -Id $p.Id -Force
        }
    }

    # 8. uninstall and confirm nothing is left behind
    if ($installed -and -not $KeepInstalled) {
        Write-Step '8. scoop uninstall'
        $code = Invoke-Scoop @('uninstall', $name)
        $left = @()
        if (Test-Path $appDir) { $left += "apps\$name" }
        if ($alias -and (Test-Path (Join-Path $scoopRoot "shims\$alias.exe"))) { $left += "shims\$alias.exe" }
        if ($shortcutName -and (Test-Path (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Scoop Apps\$shortcutName.lnk"))) { $left += 'Start-menu shortcut' }
        if ($left.Count -eq 0) { Add-Result 'scoop uninstall' 'PASS' 'app folder, shim and shortcut are gone' }
        else { Add-Result 'scoop uninstall' 'FAIL' ('left behind: ' + ($left -join ', ')) }
    } elseif ($installed) {
        Add-Result 'scoop uninstall' 'SKIP' 'kept installed (-KeepInstalled)'
    }
    if ($Bucket -and -not $KeepInstalled) { [void](Invoke-Scoop @('bucket', 'rm', $bucketName)) }
} catch {
    Add-Result 'script error' 'FAIL' $_.Exception.Message
} finally {
    Write-Step 'Summary'
    $table = ($results | Format-Table -AutoSize -Wrap | Out-String -Width 200).TrimEnd()
    Write-Host $table
    $failed = @($results | Where-Object { $_.Status -eq 'FAIL' }).Count
    $overall = 'PASS'
    if ($failed -gt 0) { $overall = 'FAIL' }
    Write-Host ''
    Write-Host "OVERALL: $overall" -ForegroundColor $(if ($failed -eq 0) { 'Green' } else { 'Red' })
    $text = "OVERALL: {0}`r`n`r`nScoop test: {1} {2}`r`n{3}`r`n" -f $overall, $name, $version, $table
    $text | Set-Content -Path (Join-Path $logDir 'latest-scoop.txt') -Encoding ASCII
    $text | Set-Content -Path (Join-Path $logDir 'latest-result.txt') -Encoding ASCII
    Write-Host "Logs: $logDir"
    Stop-Transcript | Out-Null
    if (-not $NoPause) { Read-Host 'Press Enter to close' | Out-Null }
}
