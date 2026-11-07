@echo off
rem Builds ThreadSmith (release). Close ThreadSmith first.
rem Takes a minute or two. Details go to build_app_log.txt.
cd /d "%~dp0"
echo Building ThreadSmith...
echo STARTED %DATE% %TIME%> build_app_log.txt
cargo build --release >> build_app_log.txt 2>&1
set ERR=%ERRORLEVEL%
echo EXIT CODE %ERR%>> build_app_log.txt
if %ERR% NEQ 0 (
  echo Build FAILED. See build_app_log.txt
  if not "%1"=="nopause" pause
  exit /b %ERR%
)
echo Build OK. Starting ThreadSmith...
start "" "%~dp0target\release\threadsmith.exe"
if not "%1"=="nopause" timeout /t 3 >nul
