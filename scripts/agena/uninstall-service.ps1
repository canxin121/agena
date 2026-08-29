Param(
  [string]$InstallDir = "$HOME\agena",
  [switch]$RemoveInstallDir
)

# Remove the Agena user scheduled task and optionally its files.

$ErrorActionPreference = "Stop"

try {
  Unregister-ScheduledTask -TaskName "Agena" -Confirm:$false -ErrorAction Stop
}
catch {}

Get-Process agena -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

if ($RemoveInstallDir -and (Test-Path -LiteralPath $InstallDir)) {
  Remove-Item -LiteralPath $InstallDir -Recurse -Force
}

Write-Host "Agena background task removed."
