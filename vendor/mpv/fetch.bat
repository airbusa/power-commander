@echo off
rem Downloads libmpv for Windows into vendor\mpv\windows-*, checked against SHA256SUMS.
rem Needs Windows 10 or later (curl, certutil). macOS: build.sh; Linux has the system's.
setlocal
cd /d "%~dp0"
set BUILD=20261111-git-bcde6b93cc
set URL=https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261111
set DL=%~dp0.download
if not exist "%DL%" mkdir "%DL%"

rem 7-Zip's own small console tool unpacks them.
set WANT=e80db2dad715a6c5adabe292d31ed54e11f4e9b99ca349827b50573b175891bc
call :download 7zr.exe https://github.com/ip7z/7zip/releases/download/26.06/7zr.exe || exit /b 1
call :unpack mpv-dev-x86_64-%BUILD%.7z windows-x86_64 || exit /b 1
call :unpack mpv-dev-aarch64-%BUILD%.7z windows-aarch64 || exit /b 1
rmdir /s /q "%DL%"
echo libmpv %BUILD% is in %~dp0
exit /b 0

:unpack
set WANT=
for /f "tokens=1" %%h in ('findstr /e /c:"  %1" SHA256SUMS') do set WANT=%%h
if not defined WANT (echo %1 is not in SHA256SUMS & exit /b 1)
call :download %1 %URL%/%1 || exit /b 1
if exist %2 rmdir /s /q %2
"%DL%\7zr.exe" e -y -bso0 -bsp0 -o%2 "%DL%\%1" libmpv-2.dll || exit /b 1
exit /b 0

:download
curl -fsSL -o "%DL%\%1" "%2" || (echo Could not download %1 & exit /b 1)
certutil -hashfile "%DL%\%1" SHA256 | findstr /i /x "%WANT%" >nul || (echo %1: checksum differs & exit /b 1)
exit /b 0
