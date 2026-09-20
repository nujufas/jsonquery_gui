<#
.SYNOPSIS
    Tests the Chocolatey package end to end on this Windows VM.

.DESCRIPTION
    Installs Chocolatey if it is missing, installs the staged .nupkg from a local folder (so
    the exact file that will be pushed is what gets tested), verifies the download checksum
    was checked and what landed on disk (exe, `jsonquery-gui` shim, Start-menu shortcut),
    optionally launches the app, uninstalls and confirms nothing is left behind.

    Chocolatey needs an elevated shell; the script relaunches itself as Administrator.

    A transcript, Chocolatey's own log and latest-choco.txt / latest-result.txt are written
    to .\logs next to this script (<WINVM_DIR>/shared/logs on the Linux host, see
    windows-vm.sh, which stages this script and the .nupkg into the share).

    Start it by double-clicking test-choco-package.cmd, or from any prompt:
      powershell -ExecutionPolicy Bypass -File \\host.lan\Data\test-choco-package.ps1

.PARAMETER Package
    The .nupkg to test. Default: the newest chocolatey\*.nupkg next to this script.

.PARAMETER KeepInstalled
    Leave the package installed at the end instead of uninstalling it.

.PARAMETER Launch
    Also start the installed app and report whether its process stays alive.

.PARAMETER NoPause
    Do not wait for Enter at the end.
#>
[CmdletBinding()]
param(
    [string]$Package,
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

$root = Get-UncPath $PSScriptRoot
$logDir = Join-Path $root 'logs'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$logFile = Join-Path $logDir ('choco-test-{0}.log' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
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

$script:chocoOut = ''
function Invoke-Choco([string[]]$ChocoArgs) {
    # Runs choco, echoes its output, keeps it in $script:chocoOut, returns the exit code.
    Write-Host ('> choco ' + ($ChocoArgs -join ' ')) -ForegroundColor DarkGray
    $script:chocoOut = (& $script:choco @ChocoArgs 2>&1 | ForEach-Object { "$_" } | Out-String)
    Write-Host $script:chocoOut.TrimEnd()
    return $LASTEXITCODE
}

$pkgName = 'jsonquery-gui'
$chocoRoot = $env:ChocolateyInstall
if (-not $chocoRoot) { $chocoRoot = Join-Path $env:ProgramData 'chocolatey' }
$libDir = Join-Path $chocoRoot "lib\$pkgName"
$shim = Join-Path $chocoRoot "bin\$pkgName.exe"
$lnk = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'jsonquery gui.lnk'

try {
    $os = Get-CimInstance Win32_OperatingSystem
    Write-Host ''
    Write-Host 'Chocolatey package test' -ForegroundColor Cyan
    Write-Host ('  log : {0}' -f $logFile)
    Write-Host ('  OS  : {0} build {1}' -f $os.Caption, $os.BuildNumber)

    # 1. the package under test
    Write-Step '1. Package'
    if (-not $Package) {
        $found = Get-ChildItem (Join-Path $root 'chocolatey') -Filter '*.nupkg' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($found) { $Package = $found.FullName }
    }
    if (-not $Package -or -not (Test-Path $Package)) {
        Add-Result 'package file' 'FAIL' 'no .nupkg found (run dev/choco.sh pack, then windows-vm.sh stage choco; or pass -Package)'
        return
    }
    # a local copy in its own folder is the --source, and avoids network-path quirks
    $work = Join-Path $env:TEMP 'choco-test'
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    Copy-Item $Package $work -Force
    $nupkg = Get-ChildItem $work -Filter '*.nupkg' | Select-Object -First 1
    $version = ($nupkg.BaseName -replace '^jsonquery-gui\.', '')
    Add-Result 'package file' 'PASS' ('{0} ({1:N1} KB)' -f $nupkg.Name, ($nupkg.Length / 1KB))

    # 2. Chocolatey itself
    Write-Step '2. Chocolatey'
    $script:choco = Join-Path $chocoRoot 'bin\choco.exe'
    if (-not (Test-Path $script:choco)) {
        Write-Host '  Chocolatey is not installed - installing it from https://community.chocolatey.org/install.ps1'
        try {
            Set-ExecutionPolicy Bypass -Scope Process -Force
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-Expression ((New-Object System.Net.WebClient).DownloadString('https://community.chocolatey.org/install.ps1'))
        } catch {
            Add-Result 'install Chocolatey' 'FAIL' $_.Exception.Message
            return
        }
        $chocoRoot = $env:ChocolateyInstall
        if (-not $chocoRoot) { $chocoRoot = Join-Path $env:ProgramData 'chocolatey' }
        $script:choco = Join-Path $chocoRoot 'bin\choco.exe'
    }
    if (Test-Path $script:choco) {
        $chocoVer = (& $script:choco --version 2>&1 | Select-Object -First 1)
        Add-Result 'Chocolatey' 'PASS' "v$chocoVer"
    } else {
        Add-Result 'Chocolatey' 'FAIL' "choco.exe not found at $script:choco"
        return
    }

    # 3. install (removing a previous install first)
    Write-Step '3. choco install'
    if (Test-Path $libDir) {
        Write-Host '  found a previous install - removing it first'
        [void](Invoke-Choco @('uninstall', $pkgName, '-y', '--no-progress'))
    }
    $code = Invoke-Choco @('install', $pkgName, '--source', $work, '-y', '--no-progress')
    $exe = Join-Path $libDir "tools\jsonquery_gui-$version-windows-x86_64\jsonquery_gui.exe"
    $installed = ($code -eq 0 -and (Test-Path $exe))
    if ($installed) { Add-Result 'choco install' 'PASS' "exit $code" }
    else { Add-Result 'choco install' 'FAIL' "exit $code (exe expected at $exe)" }
    if ($script:chocoOut -match 'Hashes match') { Add-Result 'checksum verified' 'PASS' 'Chocolatey reported "Hashes match"' }
    elseif ($installed) { Add-Result 'checksum verified' 'FAIL' 'no "Hashes match" line in the install output - the download was not checksum-verified' }

    # 4. what actually landed on disk
    Write-Step '4. Installed files, command and shortcut'
    if ($installed) {
        $fi = Get-Item $exe
        Add-Result 'installed exe' 'PASS' ('{0} ({1:N1} MB)' -f $fi.FullName, ($fi.Length / 1MB))
        $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
        $cmd = Get-Command $pkgName -ErrorAction SilentlyContinue
        if ($cmd -and (Test-Path $shim)) { Add-Result "command '$pkgName'" 'PASS' $cmd.Source }
        else { Add-Result "command '$pkgName'" 'FAIL' "no shim at $shim" }
        if (Test-Path (Join-Path $chocoRoot 'bin\jsonquery_gui.exe')) {
            Add-Result 'no duplicate shim' 'WARN' 'bin\jsonquery_gui.exe exists as well - the .ignore file did not stop the automatic shim'
        } else {
            Add-Result 'no duplicate shim' 'PASS' 'only the jsonquery-gui command was shimmed'
        }
        if (Test-Path $lnk) {
            $lnkTarget = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk).TargetPath
            if ($lnkTarget -eq $exe) { Add-Result 'Start-menu shortcut' 'PASS' "jsonquery gui -> $lnkTarget" }
            else { Add-Result 'Start-menu shortcut' 'FAIL' "points at $lnkTarget, expected $exe" }
        } else {
            Add-Result 'Start-menu shortcut' 'FAIL' "missing: $lnk"
        }
        $listed = (& $script:choco list 2>&1 | ForEach-Object { "$_" } | Where-Object { $_ -match "^$pkgName " } | Select-Object -First 1)
        if ($listed) { Add-Result 'choco list' 'PASS' $listed } else { Add-Result 'choco list' 'FAIL' "$pkgName is not listed as installed" }
        $sig = Get-AuthenticodeSignature $exe
        if ($sig.Status -eq 'Valid') { Add-Result 'Authenticode' 'PASS' "signed by $($sig.SignerCertificate.Subject)" }
        else { Add-Result 'Authenticode' 'WARN' "$($sig.Status) - unsigned exe, so expect SmartScreen/reputation warnings" }
    }

    # 5. optional: does the GUI start? (this VM has no GPU, so a failure here is not the package's fault)
    if ($Launch -and $installed) {
        Write-Step '5. Launch'
        $p = Start-Process -FilePath $exe -PassThru
        Start-Sleep -Seconds 8
        if ($p.HasExited) {
            Add-Result 'launch' 'WARN' "exited with code $($p.ExitCode) within 8 s (no GPU in this VM, so OpenGL may be unavailable)"
        } else {
            Add-Result 'launch' 'PASS' 'process still running after 8 s (closing it)'
            Stop-Process -Id $p.Id -Force
        }
    }

    # 6. uninstall and confirm nothing is left behind
    if ($installed -and -not $KeepInstalled) {
        Write-Step '6. choco uninstall'
        $code = Invoke-Choco @('uninstall', $pkgName, '-y', '--no-progress')
        $left = @()
        if (Test-Path $libDir) { $left += "lib\$pkgName" }
        if (Test-Path $shim) { $left += 'bin\jsonquery-gui.exe' }
        if (Test-Path $lnk) { $left += 'Start-menu shortcut' }
        if ($code -eq 0 -and $left.Count -eq 0) { Add-Result 'choco uninstall' 'PASS' 'package folder, command and shortcut are gone' }
        elseif ($left.Count -eq 0) { Add-Result 'choco uninstall' 'WARN' "exit $code, but nothing is left behind" }
        else { Add-Result 'choco uninstall' 'FAIL' ("exit $code, left behind: " + ($left -join ', ')) }
    } elseif ($installed) {
        Add-Result 'choco uninstall' 'SKIP' 'kept installed (-KeepInstalled)'
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
    $text = "OVERALL: {0}`r`n`r`nChocolatey test: {1} {2}`r`n{3}`r`n" -f $overall, $pkgName, $version, $table
    $text | Set-Content -Path (Join-Path $logDir 'latest-choco.txt') -Encoding ASCII
    $text | Set-Content -Path (Join-Path $logDir 'latest-result.txt') -Encoding ASCII
    # Chocolatey's own log goes next to ours
    Copy-Item (Join-Path $chocoRoot 'logs\chocolatey.log') (Join-Path $logDir 'chocolatey.log') -Force -ErrorAction SilentlyContinue
    Write-Host "Logs: $logDir"
    Stop-Transcript | Out-Null
    if (-not $NoPause) { Read-Host 'Press Enter to close' | Out-Null }
}
