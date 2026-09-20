$ErrorActionPreference = 'Stop'

# The extracted files go with the package folder; the command and the shortcut were made by
# chocolateyInstall.ps1 and have to be removed here.
Uninstall-BinFile -Name 'jsonquery-gui'
Remove-Item (Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'jsonquery gui.lnk') -Force -ErrorAction SilentlyContinue
