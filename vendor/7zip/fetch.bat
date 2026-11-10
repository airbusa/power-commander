@echo off
rem Downloads the bundled 7-Zip into vendor\7zip\windows-*, checked against SHA256SUMS.
rem Needs Windows 10 or later (curl, certutil). Linux and macOS: fetch.sh.
setlocal
cd /d "%~dp0"
set VERSION=26.06
set TAG=2606
set URL=https://github.com/ip7z/7zip/releases/download/%VERSION%
set DL=%~dp0.download
if not exist "%DL%" mkdir "%DL%"

rem 7zr.exe, 7-Zip's own small console tool, unpacks the installers.
call :get 7zr.exe || exit /b 1
call :unpack 7z%TAG%-x64.exe windows-x86_64 || exit /b 1
call :unpack 7z%TAG%-arm64.exe windows-aarch64 || exit /b 1
rmdir /s /q "%DL%"
echo 7-Zip %VERSION% is in %~dp0
exit /b 0

:unpack
call :get %1 || exit /b 1
if exist %2 rmdir /s /q %2
"%DL%\7zr.exe" e -y -bso0 -bsp0 -o%2 "%DL%\%1" 7z.exe 7z.dll License.txt || exit /b 1
exit /b 0

:get
set WANT=
for /f "tokens=1" %%h in ('findstr /e /c:"  %1" SHA256SUMS') do set WANT=%%h
if not defined WANT (echo %1 is not in SHA256SUMS & exit /b 1)
curl -fsSL -o "%DL%\%1" "%URL%/%1" || (echo Could not download %1 & exit /b 1)
certutil -hashfile "%DL%\%1" SHA256 | findstr /i /x "%WANT%" >nul || (echo %1: checksum differs & exit /b 1)
exit /b 0
