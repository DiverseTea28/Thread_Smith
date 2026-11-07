@echo off
rem Builds the ThreadSmith Bridge plugin and installs it into your project.
rem Usage: UpdatePlugin.bat ["C:\path\to\YourProject"]   (default: Cave_Game)
setlocal
set "HERE=%~dp0"
set "PROJECT=%~1"
if "%PROJECT%"=="" set "PROJECT=%USERPROFILE%\Documents\Unreal Projects\Cave_Game"

echo Step 1 of 3: building the plugin (a few minutes, Unreal can stay open)...
call "%HERE%BuildPlugin.bat" nopause
if errorlevel 1 (
  echo.
  echo The build failed. Details are in "%HERE%build_log.txt".
  pause
  exit /b 1
)

echo.
echo Step 2 of 3: installing. Unreal has to be closed for this.
:waitclose
tasklist /FI "IMAGENAME eq UnrealEditor.exe" 2>nul | find /I "UnrealEditor.exe" >nul
if not errorlevel 1 (
  echo    Unreal is still open. Save your work, close Unreal, then press a key here.
  pause >nul
  goto waitclose
)
call "%HERE%InstallToProject.bat" "%PROJECT%" nopause

echo.
echo Step 3 of 3: opening your project again...
for %%F in ("%PROJECT%\*.uproject") do start "" "%%~fF"
echo Done.
timeout /t 5 >nul
