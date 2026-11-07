@echo off
rem Copies the built ThreadSmith Bridge plugin into an Unreal project's Plugins folder.
rem Usage: InstallToProject.bat "C:\path\to\YourProject"   (folder that holds the .uproject)
rem With no argument it installs into Tyler's Cave_Game project.
rem Close the Unreal editor for that project first, or restart it afterwards.
setlocal
set "HERE=%~dp0"
set "PROJECT=%~1"
if "%PROJECT%"=="" set "PROJECT=%USERPROFILE%\Documents\Unreal Projects\Cave_Game"
if not exist "%HERE%Build\ThreadSmithBridge\ThreadSmithBridge.uplugin" (
  echo Build the plugin first: BuildPlugin.bat
  pause
  exit /b 1
)
echo Installing into "%PROJECT%\Plugins\ThreadSmithBridge"
robocopy "%HERE%Build\ThreadSmithBridge" "%PROJECT%\Plugins\ThreadSmithBridge" /MIR /NFL /NDL /NJH /NJS
if %ERRORLEVEL% GEQ 8 (
  echo Copy FAILED.
) else (
  echo Done. Start the project; look for Window ^> ThreadSmith Wardrobe.
)
if "%2"=="nopause" exit /b 0
pause
