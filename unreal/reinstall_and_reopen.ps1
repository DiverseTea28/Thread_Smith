# Waits for Unreal to close, installs the freshly built ThreadSmith Bridge, then reopens the project.
param([int]$EditorPid)
$root = Split-Path -Parent $PSScriptRoot
$project = Join-Path $env:USERPROFILE 'Documents\Unreal Projects\Cave_Game'
Wait-Process -Id $EditorPid -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
& cmd /c "`"$PSScriptRoot\InstallToProject.bat`" `"$project`" nopause" *> (Join-Path $PSScriptRoot 'install_log.txt')
Start-Process 'C:\Program Files\Epic Games\UE_5.8\Engine\Binaries\Win64\UnrealEditor.exe' -ArgumentList "`"$project\GameAnimationSample.uproject`""
