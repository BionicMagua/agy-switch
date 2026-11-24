param([switch]$Launch)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path $PSScriptRoot -Parent
$source = Join-Path $workspace 'src-tauri\target\release\agy-switch.exe'
$destinationDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\agy-switch'
[IO.Directory]::CreateDirectory($destinationDir) | Out-Null
$destination = Join-Path $destinationDir 'agy-switch.exe'
. (Join-Path $PSScriptRoot 'write-release-exe.ps1')
Write-ReleaseExecutable -Source $source -Destination $destination
Write-Output $destination
if ($Launch) { Start-Process -FilePath $destination }
