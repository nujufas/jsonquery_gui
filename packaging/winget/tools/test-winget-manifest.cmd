@echo off
rem Double-click launcher for test-winget-manifest.ps1 (bypasses the execution policy).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0test-winget-manifest.ps1" %*
