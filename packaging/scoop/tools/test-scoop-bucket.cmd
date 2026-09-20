@echo off
rem Double-click launcher: test the PUBLISHED Scoop bucket (adds it, installs jsonquery-gui from it,
rem uninstalls, removes the bucket again). test-scoop-manifest.cmd tests the local manifest instead.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0test-scoop-manifest.ps1" -Bucket https://github.com/nujufas/scoop-jsonquery-gui %*
