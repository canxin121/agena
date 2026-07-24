Param(
  [string]$Archive = "",
  [string]$Repo = "canxin121/agena",
  [string]$Version = "",
  [string]$InstallDir = "$HOME\agena\studio",
  [string]$ListenHost = "127.0.0.1",
  [int]$Port = 3210,
  [string]$UiPassword = "",
  [string]$WorkspaceRoot = "",
  [string]$DatabasePath = "",
  [string]$DatabaseUrl = "",
  [string[]]$Set = @()
)

# Install the released Agena Studio backend as a user scheduled task.

$ErrorActionPreference = "Stop"

function Quote-Arg([string]$Value) {
  if ($Value -match '[\s"]') {
    return '"' + ($Value -replace '"', '\"') + '"'
  }
  return $Value
}

function Normalize-Version([string]$Raw) {
  if ($Raw.StartsWith("v")) {
    return $Raw.Substring(1)
  }
  return $Raw
}

if (-not $Archive) {
  if (-not $Version) {
    throw "Provide -Archive or -Version"
  }
  $NormalizedVersion = Normalize-Version $Version
  $Archive = "https://github.com/$Repo/releases/download/agena-studio-v$NormalizedVersion/agena-studio-backend-x86_64-pc-windows-msvc-v$NormalizedVersion.zip"
}

$TempDir = Join-Path $env:TEMP ("agena-studio-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

try {
  $ArchivePath = Join-Path $TempDir "backend.zip"
  if ($Archive -match '^https?://') {
    Invoke-WebRequest -Uri $Archive -OutFile $ArchivePath
  }
  else {
    Copy-Item -LiteralPath $Archive -Destination $ArchivePath -Force
  }

  $StageDir = Join-Path $TempDir "extract"
  Expand-Archive -LiteralPath $ArchivePath -DestinationPath $StageDir -Force

  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  foreach ($name in @("bin", "web-dist", "logs")) {
    $path = Join-Path $InstallDir $name
    if (Test-Path -LiteralPath $path) {
      Remove-Item -LiteralPath $path -Recurse -Force
    }
  }

  Copy-Item -LiteralPath (Join-Path $StageDir "bin") -Destination (Join-Path $InstallDir "bin") -Recurse -Force
  Copy-Item -LiteralPath (Join-Path $StageDir "web-dist") -Destination (Join-Path $InstallDir "web-dist") -Recurse -Force
  New-Item -ItemType Directory -Force -Path (Join-Path $InstallDir "logs") | Out-Null

  $BinaryPath = Join-Path $InstallDir "bin\agena-studio.exe"
  if (-not (Test-Path -LiteralPath $BinaryPath)) {
    throw "Installed binary not found: $BinaryPath"
  }

  $ArgList = @(
    "--host", $ListenHost,
    "--port", "$Port",
    "--ui-dir", (Join-Path $InstallDir "web-dist")
  )
  if ($UiPassword) { $ArgList += @("--ui-password", $UiPassword) }
  if ($WorkspaceRoot) { $ArgList += @("--workspace-root", $WorkspaceRoot) }
  if ($DatabasePath) { $ArgList += @("--database-path", $DatabasePath) }
  if ($DatabaseUrl) { $ArgList += @("--database-url", $DatabaseUrl) }
  foreach ($item in $Set) {
    $ArgList += @("--set", $item)
  }

  $ArgumentString = ($ArgList | ForEach-Object { Quote-Arg $_ }) -join " "
  Set-Content -LiteralPath (Join-Path $InstallDir "launch-args.txt") -Value $ArgumentString

  $TaskName = "AgenaStudio"
  try {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction Stop
  }
  catch {}

  $Action = New-ScheduledTaskAction -Execute $BinaryPath -Argument $ArgumentString
  $Trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
  $Settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew
  Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Settings $Settings -Description "Agena Studio background backend"

  Start-Process -FilePath $BinaryPath -ArgumentList $ArgList -WindowStyle Hidden

  Write-Host "Installed Agena Studio background task: $TaskName"
}
finally {
  if (Test-Path -LiteralPath $TempDir) {
    Remove-Item -LiteralPath $TempDir -Recurse -Force
  }
}
