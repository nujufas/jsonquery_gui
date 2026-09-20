<#
.SYNOPSIS
    Wraps the released Windows exe into an MSIX package.

.DESCRIPTION
    Takes the release's jsonquery_gui-<version>-windows-x86_64.zip (the very bytes winget, Scoop
    and Chocolatey install), stages the exe with the icon assets and the rendered manifest, and
    packs it with MakeAppx from the Windows SDK.

    Flavor Store: identity from STORE_* in identity.env (Partner Center's Product identity),
                  written UNSIGNED - the Store re-signs it after certification. Exits with
                  code 3 (and builds nothing) while STORE_* is still empty.
    Flavor Test : identity from DEV_*, signed with a throwaway self-signed certificate, for
                  sideload tests (test-msix-package.ps1 in a Windows VM). Writes the .cer too.

    Runs in Windows PowerShell 5.1 with the Windows SDK's makeappx and signtool (the GitHub windows
    runner has them; on the Docker Windows VM: winget install Microsoft.WindowsSDK.10.0.26100).
    See README.md.

.PARAMETER Flavor
    Store or Test.

.PARAMETER ZipPath
    The release zip. Its name supplies the version unless -Version is given.

.PARAMETER Version
    x.y.z; the package version becomes x.y.z.0 (the Store reserves the 4th part).

.PARAMETER OutDir
    Where the package (and for Test the .cer) goes. Default .\dist\msix.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('Store', 'Test')][string]$Flavor,
    [Parameter(Mandatory = $true)][string]$ZipPath,
    [string]$Version,
    [string]$OutDir = '.\dist\msix',
    [string]$IdentityFile = (Join-Path $PSScriptRoot 'identity.env')
)

$ErrorActionPreference = 'Stop'

function Read-EnvFile([string]$Path) {
    $map = @{}
    foreach ($line in Get-Content $Path) {
        if ($line -match '^\s*([A-Z0-9_]+)\s*=\s*(.*?)\s*$') { $map[$Matches[1]] = $Matches[2] }
    }
    return $map
}

function Find-SdkTool([string]$Name) {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $found = Get-ChildItem $kits -Recurse -Filter $Name -ErrorAction SilentlyContinue |
        Where-Object { $_.DirectoryName -match '\\10\.0\.\d+\.\d+\\x64$' } |
        Sort-Object { [version](Split-Path (Split-Path $_.DirectoryName -Parent) -Leaf) } -Descending | Select-Object -First 1
    if ($found) { return $found.FullName }
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    throw "$Name not found under $kits or on PATH - install the Windows SDK (winget install Microsoft.WindowsSDK.10.0.26100)"
}

$cfg = Read-EnvFile $IdentityFile
if ($Flavor -eq 'Store') {
    $name = $cfg['STORE_IDENTITY_NAME']
    $publisher = $cfg['STORE_PUBLISHER']
    $publisherDisplay = $cfg['STORE_PUBLISHER_DISPLAY_NAME']
    if (-not $name -or -not $publisher -or -not $publisherDisplay) {
        Write-Host 'STORE_* in identity.env is still empty (reserve the app name in Partner Center and paste Product identity) - nothing built.'
        exit 3
    }
} else {
    $name = $cfg['DEV_IDENTITY_NAME']
    $publisher = $cfg['DEV_PUBLISHER']
    $publisherDisplay = $cfg['DEV_PUBLISHER_DISPLAY_NAME']
}
$displayName = $cfg['DISPLAY_NAME']

if (-not (Test-Path $ZipPath)) { throw "release zip not found: $ZipPath" }
if (-not $Version) {
    if ((Split-Path $ZipPath -Leaf) -match '(\d+\.\d+\.\d+)') { $Version = $Matches[1] }
    else { throw 'cannot read the version from the zip name - pass -Version x.y.z' }
}
$pkgVersion = "$Version.0"

$stage = Join-Path ([IO.Path]::GetTempPath()) ('msix-stage-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $stage | Out-Null
$unzip = Join-Path $stage '_unzip'
try {
    Expand-Archive -Path $ZipPath -DestinationPath $unzip -Force
    $exe = Get-ChildItem $unzip -Recurse -Filter 'jsonquery_gui.exe' | Select-Object -First 1
    if (-not $exe) { throw "jsonquery_gui.exe is not inside $ZipPath" }
    $root = Join-Path $stage 'pkg'
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    Copy-Item $exe.FullName (Join-Path $root 'jsonquery_gui.exe')
    Copy-Item (Join-Path $PSScriptRoot 'Assets') (Join-Path $root 'Assets') -Recurse

    $manifest = Get-Content (Join-Path $PSScriptRoot 'AppxManifest.xml') -Raw
    $values = @{
        IDENTITY_NAME = $name; PUBLISHER = $publisher; PUBLISHER_DISPLAY_NAME = $publisherDisplay
        DISPLAY_NAME = $displayName; VERSION = $pkgVersion
    }
    foreach ($key in $values.Keys) { $manifest = $manifest.Replace('{{' + $key + '}}', [Security.SecurityElement]::Escape($values[$key])) }
    if ($manifest -match '\{\{') { throw 'unresolved {{token}} left in the manifest' }
    [IO.File]::WriteAllText((Join-Path $root 'AppxManifest.xml'), $manifest, (New-Object Text.UTF8Encoding($false)))

    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $suffix = ''
    if ($Flavor -eq 'Test') { $suffix = '-test' }
    $msix = Join-Path (Resolve-Path $OutDir) ("jsonquery-gui_${pkgVersion}_x64$suffix.msix")
    $makeappx = Find-SdkTool 'makeappx.exe'
    Write-Host "makeappx: $makeappx"
    & $makeappx pack /d $root /p $msix /o
    if ($LASTEXITCODE -ne 0) { throw "makeappx failed with exit code $LASTEXITCODE" }

    if ($Flavor -eq 'Test') {
        $signtool = Find-SdkTool 'signtool.exe'
        $cer = Join-Path (Resolve-Path $OutDir) 'jsonquery-gui-test.cer'
        $pfx = Join-Path $stage 'test.pfx'
        $password = [guid]::NewGuid().ToString()
        $cert = New-SelfSignedCertificate -Type Custom -Subject $publisher -KeyUsage DigitalSignature `
            -FriendlyName 'jsonquery gui test package' -CertStoreLocation 'Cert:\CurrentUser\My' `
            -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
        try {
            [void](Export-Certificate -Cert $cert -FilePath $cer)
            [void](Export-PfxCertificate -Cert $cert -FilePath $pfx -Password (ConvertTo-SecureString -String $password -AsPlainText -Force))
            & $signtool sign /fd SHA256 /f $pfx /p $password $msix
            if ($LASTEXITCODE -ne 0) { throw "signtool failed with exit code $LASTEXITCODE" }
        } finally {
            Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -ErrorAction SilentlyContinue
        }
        Write-Host "certificate (public part) : $cer"
    }

    $item = Get-Item $msix
    Write-Host ''
    Write-Host "package  : $($item.FullName)"
    Write-Host ('size     : {0:N1} MB' -f ($item.Length / 1MB))
    Write-Host "sha256   : $((Get-FileHash $msix -Algorithm SHA256).Hash)"
    Write-Host "identity : $name  $publisher  version $pkgVersion  ($Flavor)"
} finally {
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}
