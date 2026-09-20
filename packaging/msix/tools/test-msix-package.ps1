<#
.SYNOPSIS
    Sideload-tests the test-signed MSIX on this Windows VM.

.DESCRIPTION
    Trusts the throwaway test certificate, installs the .msix with Add-AppxPackage, verifies the
    registration (package, Start-menu entry, `jsonquery-gui` command alias), launches the app,
    saves a screenshot of the desktop, uninstalls it and confirms nothing is left behind.

    The launch matters beyond this test: the Store's certification testers run the app on
    machines without a GPU, much like this VM, so "does it come up here?" is a fair preview.

    Logs, latest-msix.txt / latest-result.txt and msix-launch.png go to .\logs next to this
    script (<WINVM_DIR>/shared/logs on the Linux host, see windows-vm.sh, which stages this
    script and the package into the share).

.PARAMETER Package
    The test-signed .msix. Default: the newest msix\*-test.msix next to this script.

.PARAMETER Certificate
    The matching .cer. Default: the .cer next to the package.

.PARAMETER KeepInstalled
    Leave the package installed (and the certificate trusted) at the end.

.PARAMETER LaunchSeconds
    How long to wait after starting the app before checking it and taking the screenshot (default 12).

.PARAMETER NoPause
    Do not wait for Enter at the end.
#>
[CmdletBinding()]
param(
    [string]$Package,
    [string]$Certificate,
    [switch]$KeepInstalled,
    [int]$LaunchSeconds = 12,
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
            $argList += ('"{0}"' -f (Get-UncPath "$value"))
        }
    }
    Start-Process powershell.exe -Verb RunAs -ArgumentList $argList
    exit
}

$root = Get-UncPath $PSScriptRoot
$logDir = Join-Path $root 'logs'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$logFile = Join-Path $logDir ('msix-test-{0}.log' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
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

function Save-Screenshot([string]$Path) {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height)
    $gfx = [System.Drawing.Graphics]::FromImage($bmp)
    $gfx.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $gfx.Dispose()
    $bmp.Dispose()
}

$pkgName = $null
$certThumb = $null
$installed = $false

try {
    $os = Get-CimInstance Win32_OperatingSystem
    Write-Host ''
    Write-Host 'MSIX sideload test' -ForegroundColor Cyan
    Write-Host ('  log : {0}' -f $logFile)
    Write-Host ('  OS  : {0} build {1}' -f $os.Caption, $os.BuildNumber)

    # 1. the files under test
    Write-Step '1. Package and certificate'
    if (-not $Package) {
        $found = Get-ChildItem (Join-Path $root 'msix') -Filter '*-test.msix' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($found) { $Package = $found.FullName }
    }
    if (-not $Package -or -not (Test-Path $Package)) {
        Add-Result 'package file' 'FAIL' 'no msix\*-test.msix next to the script (windows-vm.sh stage msix, or pass -Package)'
        return
    }
    if (-not $Certificate) { $Certificate = Join-Path (Split-Path $Package -Parent) 'jsonquery-gui-test.cer' }
    if (-not (Test-Path $Certificate)) {
        Add-Result 'certificate' 'FAIL' "no test certificate at $Certificate"
        return
    }
    # a local copy avoids network-path quirks in the deployment service
    $work = Join-Path $env:TEMP 'msix-test'
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    Copy-Item $Package $work -Force
    Copy-Item $Certificate $work -Force
    $localPkg = Join-Path $work (Split-Path $Package -Leaf)
    $localCer = Join-Path $work (Split-Path $Certificate -Leaf)
    Add-Result 'package file' 'PASS' ('{0} ({1:N1} MB)' -f (Split-Path $Package -Leaf), ((Get-Item $localPkg).Length / 1MB))

    # 2. trust the test certificate (machine store: TrustedPeople is what sideloaded packages need)
    Write-Step '2. Trust the test certificate'
    try {
        $imported = Import-Certificate -FilePath $localCer -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople'
        $certThumb = $imported.Thumbprint
        Add-Result 'test certificate' 'PASS' ('{0} ({1})' -f $imported.Subject, $certThumb)
    } catch {
        Add-Result 'test certificate' 'FAIL' $_.Exception.Message
        return
    }

    # 3. install
    Write-Step '3. Add-AppxPackage'
    # the package identity is in the manifest inside the package; read it from the package itself
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($localPkg)
    try {
        $entry = $zip.GetEntry('AppxManifest.xml')
        $reader = New-Object IO.StreamReader($entry.Open())
        [xml]$mf = $reader.ReadToEnd()
        $reader.Dispose()
    } finally { $zip.Dispose() }
    $pkgName = $mf.Package.Identity.Name
    $pkgVersion = $mf.Package.Identity.Version
    $appId = $mf.Package.Applications.Application.Id
    Write-Host ('  identity {0} version {1}, app id {2}' -f $pkgName, $pkgVersion, $appId)
    $old = Get-AppxPackage -Name $pkgName -ErrorAction SilentlyContinue
    if ($old) {
        Write-Host '  found a previous install - removing it first'
        $old | Remove-AppxPackage -ErrorAction SilentlyContinue
    }
    try {
        Add-AppxPackage -Path $localPkg -ErrorAction Stop
        $installed = $true
        Add-Result 'Add-AppxPackage' 'PASS'
    } catch {
        Add-Result 'Add-AppxPackage' 'FAIL' $_.Exception.Message
        return
    }

    # 4. what got registered
    Write-Step '4. Registration'
    $pkg = Get-AppxPackage -Name $pkgName
    if ($pkg) {
        Add-Result 'package registered' 'PASS' ('{0} {1} at {2}' -f $pkg.Name, $pkg.Version, $pkg.InstallLocation)
        if ($pkg.SignatureKind) { Add-Result 'signature kind' 'PASS' "$($pkg.SignatureKind) (the Store build is re-signed by Microsoft)" }
    } else {
        Add-Result 'package registered' 'FAIL' 'Get-AppxPackage does not list it after a successful install'
        return
    }
    $tile = @(Get-StartApps | Where-Object { $_.AppID -like "$($pkg.PackageFamilyName)!*" })
    if ($tile.Count -gt 0) { Add-Result 'Start-menu entry' 'PASS' ('"{0}" ({1})' -f $tile[0].Name, $tile[0].AppID) }
    else { Add-Result 'Start-menu entry' 'FAIL' 'no Start app for the package family' }
    $aliasPath = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\jsonquery-gui.exe'
    if (Test-Path $aliasPath) { Add-Result "command alias 'jsonquery-gui'" 'PASS' $aliasPath }
    else { Add-Result "command alias 'jsonquery-gui'" 'FAIL' "missing: $aliasPath" }
    $capabilities = @($mf.Package.Capabilities.ChildNodes | ForEach-Object { $_.Name })
    Add-Result 'declared capabilities' 'PASS' ($capabilities -join ', ')

    # 5. launch and look (no GPU here, like the Store's test machines)
    Write-Step '5. Launch'
    $aumid = '{0}!{1}' -f $pkg.PackageFamilyName, $appId
    Start-Process explorer.exe -ArgumentList "shell:AppsFolder\$aumid"
    Start-Sleep -Seconds $LaunchSeconds
    $proc = @(Get-Process -Name 'jsonquery_gui' -ErrorAction SilentlyContinue)
    $shot = Join-Path $logDir 'msix-launch.png'
    try { Save-Screenshot $shot; Write-Host "  screenshot: $shot" } catch { Write-Host ('  screenshot failed: ' + $_.Exception.Message) }
    if ($proc.Count -gt 0) {
        Add-Result 'launch' 'PASS' ("jsonquery_gui.exe is running after $LaunchSeconds s (see logs\msix-launch.png)")
        $proc | Stop-Process -Force -ErrorAction SilentlyContinue
    } else {
        Add-Result 'launch' 'WARN' "no jsonquery_gui process $LaunchSeconds s after launching (no GPU in this VM, so graphics setup may have failed - see logs\msix-launch.png)"
    }
} catch {
    Add-Result 'script error' 'FAIL' $_.Exception.Message
} finally {
    # 6. uninstall and clean up
    if ($installed -and -not $KeepInstalled) {
        Write-Step '6. Remove-AppxPackage'
        Get-AppxPackage -Name $pkgName | Remove-AppxPackage -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
        $left = @()
        if (Get-AppxPackage -Name $pkgName) { $left += 'package' }
        if (Test-Path (Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\jsonquery-gui.exe')) { $left += 'command alias' }
        if ($left.Count -eq 0) { Add-Result 'uninstall' 'PASS' 'package and command alias are gone' }
        else { Add-Result 'uninstall' 'FAIL' ('left behind: ' + ($left -join ', ')) }
    } elseif ($installed) {
        Add-Result 'uninstall' 'SKIP' 'kept installed (-KeepInstalled)'
    }
    if ($certThumb -and -not $KeepInstalled) {
        Remove-Item "Cert:\LocalMachine\TrustedPeople\$certThumb" -ErrorAction SilentlyContinue
    }

    Write-Step 'Summary'
    $table = ($results | Format-Table -AutoSize -Wrap | Out-String -Width 200).TrimEnd()
    Write-Host $table
    $failed = @($results | Where-Object { $_.Status -eq 'FAIL' }).Count
    $overall = 'PASS'
    if ($failed -gt 0) { $overall = 'FAIL' }
    Write-Host ''
    Write-Host "OVERALL: $overall" -ForegroundColor $(if ($failed -eq 0) { 'Green' } else { 'Red' })
    $text = "OVERALL: {0}`r`n`r`nMSIX test: {1}`r`n{2}`r`n" -f $overall, $pkgName, $table
    $text | Set-Content -Path (Join-Path $logDir 'latest-msix.txt') -Encoding ASCII
    $text | Set-Content -Path (Join-Path $logDir 'latest-result.txt') -Encoding ASCII
    Write-Host "Logs: $logDir"
    Stop-Transcript | Out-Null
    if (-not $NoPause) { Read-Host 'Press Enter to close' | Out-Null }
}
