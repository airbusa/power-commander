@echo off
rem Downloads the bundled PDFium into vendor\pdfium\windows-*, checked against SHA256SUMS.
rem Needs Windows 10 or later (curl, certutil, tar). Linux and macOS: fetch.sh.
setlocal
cd /d "%~dp0"
set VERSION=8125
set URL=https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/%VERSION%
set DL=%~dp0.download
if not exist "%DL%" mkdir "%DL%"

call :unpack pdfium-win-x64.tgz windows-x86_64 || exit /b 1
call :unpack pdfium-win-arm64.tgz windows-aarch64 || exit /b 1
rmdir /s /q "%DL%"
echo PDFium %VERSION% is in %~dp0
exit /b 0

:unpack
call :get %1 || exit /b 1
if exist %2 rmdir /s /q %2
mkdir %2
tar -xzf "%DL%\%1" -C %2 bin/pdfium.dll LICENSE licenses || exit /b 1
move /y %2\bin\pdfium.dll %2\ >nul && rmdir %2\bin
exit /b 0

:get
set WANT=
for /f "tokens=1" %%h in ('findstr /e /c:"  %1" SHA256SUMS') do set WANT=%%h
if not defined WANT (echo %1 is not in SHA256SUMS & exit /b 1)
curl -fsSL -o "%DL%\%1" "%URL%/%1" || (echo Could not download %1 & exit /b 1)
certutil -hashfile "%DL%\%1" SHA256 | findstr /i /x "%WANT%" >nul || (echo %1: checksum differs & exit /b 1)
exit /b 0
