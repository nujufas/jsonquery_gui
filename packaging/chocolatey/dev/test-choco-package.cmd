@echo off
rem Double-click launcher for test-choco-package.ps1 (bypasses the execution policy).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0test-choco-package.ps1" %*
