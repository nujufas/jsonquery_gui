@echo off
:: Build and package the native Windows x86_64 release binary.
::
:: Unlike windows.sh (which cross-compiles from Linux/macOS via `cross` +
:: Docker), this runs directly on a Windows machine with a Rust toolchain
:: installed (https://rustup.rs) -- no Docker needed. Requires PowerShell
:: (bundled with Windows 10/Server 2016 and later) to create the zip.
setlocal

set "SCRIPT_DIR=%~dp0"
set "ROOT_DIR=%SCRIPT_DIR%.."
set "DIST_DIR=%ROOT_DIR%\dist"
set "APP_NAME=jsonquery_gui"

for /f "tokens=2 delims==" %%v in ('findstr /b /r "^version" "%ROOT_DIR%\Cargo.toml"') do set "VERSION=%%v"
set "VERSION=%VERSION: =%"
set "VERSION=%VERSION:"=%"

if not exist "%DIST_DIR%" mkdir "%DIST_DIR%"

echo ==^> Building %APP_NAME% %VERSION% (native Windows x86_64)
cargo build --release -p jsonquery_gui
if errorlevel 1 exit /b 1

set "PKG_NAME=%APP_NAME%-%VERSION%-windows-x86_64"
set "STAGE=%TEMP%\%PKG_NAME%"
if exist "%STAGE%" rmdir /s /q "%STAGE%"
mkdir "%STAGE%\%PKG_NAME%"

copy /y "%ROOT_DIR%\target\release\%APP_NAME%.exe" "%STAGE%\%PKG_NAME%\" >nul
if exist "%ROOT_DIR%\README.md" copy /y "%ROOT_DIR%\README.md" "%STAGE%\%PKG_NAME%\" >nul

set "ZIP_PATH=%DIST_DIR%\%PKG_NAME%.zip"
if exist "%ZIP_PATH%" del /f /q "%ZIP_PATH%"
powershell -NoProfile -Command "Compress-Archive -Path '%STAGE%\%PKG_NAME%' -DestinationPath '%ZIP_PATH%'"
if errorlevel 1 exit /b 1

rmdir /s /q "%STAGE%"

echo ==^> Wrote %ZIP_PATH%
endlocal
