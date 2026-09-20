<#
.SYNOPSIS
    Tests a winget manifest end to end on this Windows VM.

.DESCRIPTION
    Runs, in order: a winget version gate (installs the bundle from .\winget if the
    client is too old for the manifest's schema), enables LocalManifestFiles,
    winget validate, winget install, checks on the installed command and exe,
    Authenticode + Microsoft Defender scan, an optional launch, and winget uninstall.

    A transcript, winget's own diagnostic logs and latest-result.txt are written to
    .\logs next to this script (<WINVM_DIR>/shared/logs on the Linux host, see
    windows-vm.sh, which stages this script and the manifests into the share).

    Start it by double-clicking test-winget-manifest.cmd, or from any prompt:
      powershell -ExecutionPolicy Bypass -File \\host.lan\Data\test-winget-manifest.ps1

.PARAMETER ManifestDir
    Folder holding the manifest files. Default: the first folder next to this
    script that contains an *.installer.yaml.

.PARAMETER KeepInstalled
    Leave the package installed at the end instead of uninstalling it.

.PARAMETER Launch
    Also start the installed app and report whether its process stays alive.

.PARAMETER NoPause
    Do not wait for Enter at the end.
#>
[CmdletBinding()]
param(
    [string]$ManifestDir,
    [switch]$KeepInstalled,
    [switch]$Launch,
    [switch]$NoPause
)

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function Get-UncPath([string]$Path) {
    # An elevated process cannot see mapped drives such as Z:, so use the UNC form.
    if ($Path -match '^([A-Za-z]:)') {
        $disk = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$($Matches[1])'" -ErrorAction SilentlyContinue
        if ($disk -and $disk.ProviderName) { return $disk.ProviderName + $Path.Substring(2) }
    }
    return $Path
}

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host 'Not elevated - relaunching as Administrator (accept the UAC prompt)...'
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Get-UncPath $PSCommandPath)))
    foreach ($key in $PSBoundParameters.Keys) {
        $value = $PSBoundParameters[$key]
        if ($value -is [switch]) {
            if ($value) { $argList += "-$key" }
        } else {
            $argList += "-$key"
            $argList += ('"{0}"' -f (Get-UncPath $value))
        }
    }
    Start-Process powershell.exe -Verb RunAs -ArgumentList $argList
    exit
}

function Find-ManifestDir([string]$Root) {
    # First folder next to the script that holds an *.installer.yaml.
    $hit = Get-ChildItem $Root -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path (Join-Path $_.FullName '*.installer.yaml') } |
        Select-Object -First 1
    if ($hit) { return $hit.FullName }
    return $null
}

$root = Get-UncPath $PSScriptRoot
if (-not $ManifestDir) { $ManifestDir = Find-ManifestDir $root }
$logDir = Join-Path $root 'logs'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$logFile = Join-Path $logDir ('winget-test-{0}.log' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
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

function Format-Exit([int]$Code) { 'exit {0} (0x{1:X8})' -f $Code, $Code }

function Invoke-Winget([string[]]$WingetArgs) {
    # Runs winget, prints its output without spinner/progress-bar noise, returns the exit code.
    Write-Host ('> winget ' + ($WingetArgs -join ' ')) -ForegroundColor DarkGray
    & winget @WingetArgs 2>&1 |
        ForEach-Object { "$_" } |
        Where-Object { $_ -notmatch '^[\s\-\\|/]*$' -and $_ -notmatch '^\s*[\u2588\u2592\u2591]' } |
        ForEach-Object { Write-Host $_ }
    return $LASTEXITCODE
}

function Get-WingetVersion {
    try {
        $line = & winget --version 2>$null | Select-Object -First 1
        if ("$line" -match '(\d+\.\d+(\.\d+)*)') { return [version]$Matches[1] }
    } catch { }
    return $null
}

function Get-YamlValue([string]$File, [string]$Key) {
    $m = Select-String -Path $File -Pattern "^\s*-?\s*${Key}:\s*(\S+)" | Select-Object -First 1
    if ($m) { return $m.Matches[0].Groups[1].Value }
    return $null
}

try {
    $os = Get-CimInstance Win32_OperatingSystem
    Write-Host ''
    Write-Host 'winget manifest test' -ForegroundColor Cyan
    Write-Host ('  manifest : {0}' -f $ManifestDir)
    Write-Host ('  log      : {0}' -f $logFile)
    Write-Host ('  OS       : {0} build {1}' -f $os.Caption, $os.BuildNumber)

    $installerYaml = $null
    if ($ManifestDir -and (Test-Path $ManifestDir)) {
        $installerYaml = Get-ChildItem $ManifestDir -Filter '*.installer.yaml' | Select-Object -First 1
    }
    if (-not $installerYaml) {
        Add-Result 'manifest files' 'FAIL' 'no folder with an *.installer.yaml next to the script (or pass -ManifestDir)'
        return
    }
    $pkgId = Get-YamlValue $installerYaml.FullName 'PackageIdentifier'
    $pkgVer = Get-YamlValue $installerYaml.FullName 'PackageVersion'
    $schema = Get-YamlValue $installerYaml.FullName 'ManifestVersion'
    $alias = Get-YamlValue $installerYaml.FullName 'PortableCommandAlias'
    Add-Result 'manifest files' 'PASS' "$pkgId $pkgVer (schema $schema)"

    # 1. winget client new enough for the manifest schema?
    Write-Step '1. winget client'
    $need = [version]$schema
    $needMinor = [version]('{0}.{1}' -f $need.Major, $need.Minor)
    $have = Get-WingetVersion
    if (-not $have -or $have -lt $needMinor) {
        Write-Host ('  winget is {0}; manifest schema {1} needs >= {2}' -f $have, $schema, $needMinor)
        $bundleName = 'Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle'
        $bundle = Join-Path $root "winget\$bundleName"
        if (Test-Path $bundle) {
            # Install from a local copy: the deployment service is unreliable with network paths.
            $tmp = Join-Path $env:TEMP 'winget-update'
            New-Item -ItemType Directory -Force -Path $tmp | Out-Null
            Copy-Item $bundle $tmp -Force
            $deps = @()
            $depSrc = Join-Path $root 'winget\deps\x64'
            if (Test-Path $depSrc) {
                Copy-Item $depSrc (Join-Path $tmp 'deps') -Recurse -Force
                $deps = @(Get-ChildItem (Join-Path $tmp 'deps') -Filter '*.appx' | ForEach-Object { $_.FullName })
            }
            Write-Host '  installing the winget bundle from the share...'
            try {
                Add-AppxPackage -Path (Join-Path $tmp $bundleName) -DependencyPath $deps -ForceApplicationShutdown -ErrorAction Stop
            } catch {
                Add-Result 'update winget' 'FAIL' $_.Exception.Message
            }
            for ($i = 0; $i -lt 20; $i++) {
                $have = Get-WingetVersion
                if ($have -and $have -ge $needMinor) { break }
                Start-Sleep -Seconds 3
            }
        } else {
            Write-Host "  no bundle at $bundle"
        }
    }
    if ($have -and $have -ge $needMinor) {
        Add-Result 'winget client' 'PASS' "v$have"
    } else {
        Add-Result 'winget client' 'FAIL' "v$have is too old for schema $schema and could not be updated"
        return
    }

    # 2. local manifests are disabled by default (admin-only setting)
    Write-Step '2. Enable LocalManifestFiles'
    $code = Invoke-Winget @('settings', '--enable', 'LocalManifestFiles')
    if ($code -eq 0) { Add-Result 'enable LocalManifestFiles' 'PASS' }
    else { Add-Result 'enable LocalManifestFiles' 'WARN' ((Format-Exit $code) + ' - fine if already enabled') }

    # 3. validate
    Write-Step '3. winget validate'
    $code = Invoke-Winget @('validate', '--manifest', $ManifestDir)
    if ($code -eq 0) { Add-Result 'winget validate' 'PASS' }
    else { Add-Result 'winget validate' 'FAIL' (Format-Exit $code) }

    # 4. install (removing a previous install of the same package first)
    Write-Step '4. winget install'
    # A package installed with --manifest is not tracked by any source, so `list --id` and
    # `uninstall --id` cannot see it. Look on disk and uninstall with --manifest instead.
    $pkgRoot = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages'
    $leftover = @(Get-ChildItem $pkgRoot -Filter "$pkgId*" -ErrorAction SilentlyContinue)
    if ($leftover.Count -gt 0) {
        Write-Host '  found a previous install - removing it first'
        [void](Invoke-Winget @('uninstall', '--manifest', $ManifestDir, '--disable-interactivity'))
        $leftover = @(Get-ChildItem $pkgRoot -Filter "$pkgId*" -ErrorAction SilentlyContinue)
        if ($leftover.Count -gt 0) {
            Write-Host '  winget could not remove it - deleting the package folder and command link by hand'
            $leftover | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
            if ($alias) { Remove-Item (Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links\$alias.exe") -Force -ErrorAction SilentlyContinue }
        }
    }
    $code = Invoke-Winget @('install', '--manifest', $ManifestDir, '--accept-source-agreements', '--accept-package-agreements', '--disable-interactivity')
    $installed = ($code -eq 0)
    if ($installed) { Add-Result 'winget install' 'PASS' }
    else { Add-Result 'winget install' 'FAIL' (Format-Exit $code) }

    # 5. what actually landed on disk
    Write-Step '5. Installed command and exe'
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
    $exe = $null
    if ($installed -and $alias) {
        $cmd = Get-Command $alias -ErrorAction SilentlyContinue
        if ($cmd) {
            $exe = $cmd.Source
            $item = Get-Item $exe -Force
            if ($item.LinkType -and $item.Target) { $exe = @($item.Target)[0] }
            Add-Result "command '$alias'" 'PASS' $cmd.Source
        } else {
            Add-Result "command '$alias'" 'FAIL' 'not found on PATH after install'
        }
    }
    if ($installed -and -not $exe) {
        $found = Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages') -Recurse -Filter '*.exe' -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -like "*$pkgId*" } | Select-Object -First 1
        if ($found) { $exe = $found.FullName }
    }
    if ($exe -and (Test-Path $exe)) {
        $fi = Get-Item $exe
        Add-Result 'installed exe' 'PASS' ('{0} ({1:N1} MB)' -f $fi.FullName, ($fi.Length / 1MB))
    } elseif ($installed) {
        Add-Result 'installed exe' 'FAIL' 'exe missing after a successful install (quarantined by Defender?)'
    }

    # 6. signature and Defender (the winget pipeline runs Defender over the package)
    Write-Step '6. Signature and Defender'
    if ($exe -and (Test-Path $exe)) {
        $sig = Get-AuthenticodeSignature $exe
        if ($sig.Status -eq 'Valid') { Add-Result 'Authenticode' 'PASS' "signed by $($sig.SignerCertificate.Subject)" }
        else { Add-Result 'Authenticode' 'WARN' "$($sig.Status) - unsigned exe, so expect SmartScreen/reputation warnings" }
    }
    try {
        $st = Get-MpComputerStatus -ErrorAction Stop
        Write-Host ('  Defender before update: real-time protection {0}; signatures {1} ({2})' -f $st.RealTimeProtectionEnabled, $st.AntivirusSignatureVersion, $st.AntivirusSignatureLastUpdated)
        try { Update-MpSignature -ErrorAction Stop; Write-Host '  Update-MpSignature finished' }
        catch { Write-Host ('  could not update signatures: ' + $_.Exception.Message) }
        $st = Get-MpComputerStatus -ErrorAction Stop
        $ageDays = [int]((Get-Date) - $st.AntivirusSignatureLastUpdated).TotalDays
        Write-Host ('  Defender after update : signatures {0} ({1}, {2} days old)' -f $st.AntivirusSignatureVersion, $st.AntivirusSignatureLastUpdated, $ageDays)
        if ($exe -and (Test-Path $exe)) { Start-MpScan -ScanType CustomScan -ScanPath $exe -ErrorAction Stop }
        $hits = @(Get-MpThreatDetection -ErrorAction SilentlyContinue | Where-Object { ($_.Resources -join ' ') -match 'jsonquery' })
        if ($hits.Count -eq 0) {
            $detail = 'no detections for jsonquery (signatures {0}, {1} days old)' -f $st.AntivirusSignatureVersion, $ageDays
            if ($ageDays -gt 14) { Add-Result 'Defender scan' 'WARN' ($detail + ' - stale definitions, so this proves little') }
            else { Add-Result 'Defender scan' 'PASS' $detail }
        } else {
            $names = @($hits | ForEach-Object { (Get-MpThreat -ThreatID $_.ThreatID -ErrorAction SilentlyContinue).ThreatName } | Where-Object { $_ } | Select-Object -Unique)
            Add-Result 'Defender scan' 'FAIL' ('detected: ' + ($names -join ', '))
        }
    } catch {
        Add-Result 'Defender scan' 'WARN' ('not run: ' + $_.Exception.Message)
    }

    # 7. optional: does the GUI start? (this VM has no GPU, so a failure here is not the package's fault)
    if ($Launch -and $exe -and (Test-Path $exe)) {
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
        Write-Step '8. winget uninstall'
        $code = Invoke-Winget @('uninstall', '--manifest', $ManifestDir, '--disable-interactivity')
        if ($code -eq 0) { Add-Result 'winget uninstall' 'PASS' } else { Add-Result 'winget uninstall' 'FAIL' (Format-Exit $code) }
        $left = @(Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages') -Filter "$pkgId*" -ErrorAction SilentlyContinue)
        $linkLeft = $false
        if ($alias) { $linkLeft = Test-Path (Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links\$alias.exe") }
        if ($left.Count -eq 0 -and -not $linkLeft) { Add-Result 'clean removal' 'PASS' }
        else { Add-Result 'clean removal' 'WARN' 'package folder or command link is still present' }
    } elseif ($installed) {
        Add-Result 'winget uninstall' 'SKIP' 'kept installed (-KeepInstalled)'
    }
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
    $text = "OVERALL: {0}`r`n`r`n{1}`r`n" -f $overall, $table
    $text | Set-Content -Path (Join-Path $logDir 'latest-winget.txt') -Encoding ASCII
    $text | Set-Content -Path (Join-Path $logDir 'latest-result.txt') -Encoding ASCII

    # winget's own diagnostic logs (newest few) go next to ours
    $diag = Join-Path $env:LOCALAPPDATA 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\DiagOutputDir'
    Get-ChildItem $diag -Filter '*.log' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 3 |
        Copy-Item -Destination $logDir -Force -ErrorAction SilentlyContinue
    Write-Host "Logs: $logDir"
    Stop-Transcript | Out-Null
    if (-not $NoPause) { Read-Host 'Press Enter to close' | Out-Null }
}
