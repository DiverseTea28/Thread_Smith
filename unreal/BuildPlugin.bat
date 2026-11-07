@echo off
rem Builds the ThreadSmith Bridge plugin for Unreal 5.8 into unreal\Build\ThreadSmithBridge.
rem Takes a few minutes. Details go to unreal\build_log.txt.
setlocal
set "UE=C:\Program Files\Epic Games\UE_5.8"
set "HERE=%~dp0"
set "OUT=%HERE%Build\ThreadSmithBridge"
set "LOG=%HERE%build_log.txt"

echo Building ThreadSmith Bridge for UE 5.8 (a few minutes)...
echo STARTED %DATE% %TIME%> "%LOG%"
call "%UE%\Engine\Build\BatchFiles\RunUAT.bat" BuildPlugin -Plugin="%HERE%ThreadSmithBridge\ThreadSmithBridge.uplugin" -Package="%OUT%" -TargetPlatforms=Win64 -Rocket >> "%LOG%" 2>&1
set ERR=%ERRORLEVEL%
echo EXIT CODE %ERR%>> "%LOG%"
rem Keep a copy of Unreal's detailed compiler log next to this file.
copy /Y "%APPDATA%\Unreal Engine\AutomationTool\Logs\C+Program+Files+Epic+Games+UE_5.8\UBA-UnrealEditor-Win64-Development.txt" "%HERE%build_log_detail.txt" >nul 2>&1
if %ERR% NEQ 0 (
  echo Build FAILED. See %LOG%
) else (
  echo Build OK: %OUT%
)
if "%1"=="nopause" exit /b %ERR%
pause
