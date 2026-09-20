$ErrorActionPreference = 'Stop'

# Bump these two together with <version> in the nuspec (README.md in the package folder).
$version = '0.4.1'
$checksum64 = '9B68D2615BF3C33F65F9EAC3C713E3630CF31CC422879455FA496C0B00F2D823'

$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$folder = "jsonquery_gui-$version-windows-x86_64"

$packageArgs = @{
    packageName    = $env:ChocolateyPackageName
    unzipLocation  = $toolsDir
    url64bit       = "https://github.com/nujufas/jsonquery_gui/releases/download/v$version/$folder.zip"
    checksum64     = $checksum64
    checksumType64 = 'sha256'
}
Install-ChocolateyZipPackage @packageArgs

$exe = Join-Path $toolsDir "$folder\jsonquery_gui.exe"
if (-not (Test-Path $exe)) {
    throw "jsonquery_gui.exe was not found at $exe after extracting the release zip"
}

# One command name on every channel (winget, Scoop, Snap and Homebrew all use jsonquery-gui):
# shim it ourselves and stop Chocolatey from also shimming the same exe as jsonquery_gui.
New-Item "$exe.ignore" -ItemType File -Force | Out-Null
Install-BinFile -Name 'jsonquery-gui' -Path $exe -UseStart

Install-ChocolateyShortcut `
    -ShortcutFilePath (Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'jsonquery gui.lnk') `
    -TargetPath $exe `
    -WorkingDirectory (Split-Path $exe) `
    -Description 'Browse and query large JSON files'
