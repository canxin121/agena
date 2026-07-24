Param(
  [string]$InstallDir = "$HOME\agena\studio",
  [switch]$RemoveInstallDir
)

# Remove the Agena Studio user scheduled task and optionally its files.

$ErrorActionPreference = "Stop"

try {
  Unregister-ScheduledTask -TaskName "AgenaStudio" -Confirm:$false -ErrorAction Stop
}
catch {}

Get-Process agena-studio -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

if ($RemoveInstallDir -and (Test-Path -LiteralPath $InstallDir)) {
  Remove-Item -LiteralPath $InstallDir -Recurse -Force
}

Write-Host "Agena Studio background task removed."
