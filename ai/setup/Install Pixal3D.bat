@echo off
rem Installs Pixal3D for ThreadSmith into your ComfyUI install.
rem Use: "Install Pixal3D.bat" [ComfyUI install folder]
rem (ThreadSmith passes the folder from its settings. Without one, the usual
rem ComfyUI-Easy-Install place is used.)
setlocal
title ThreadSmith - Install Pixal3D
set "ROOT=%~1"
if "%ROOT%"=="" set "ROOT=C:\ComfyUI-Easy-Install\ComfyUI-Easy-Install"
set "PY=%ROOT%\python_embeded\python.exe"
if not exist "%PY%" set "PY=%ROOT%\python_embedded\python.exe"
if not exist "%PY%" set "PY=%ROOT%\venv\Scripts\python.exe"
if not exist "%PY%" (
  echo Could not find the ComfyUI Python in "%ROOT%".
  echo Run this with your ComfyUI install folder, for example:
  echo   "Install Pixal3D.bat" "D:\ComfyUI-Easy-Install\ComfyUI-Easy-Install"
  pause
  exit /b 1
)
echo Installing Pixal3D into %ROOT%
echo Close ComfyUI first if it is running.
echo.
"%PY%" -s "%~dp0install_pixal3d.py" "%ROOT%"
echo.
pause
