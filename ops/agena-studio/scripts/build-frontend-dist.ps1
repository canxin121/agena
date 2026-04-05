Param()

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../../..")
$WebDir = Join-Path $RepoRoot "packages/agena-studio-web"

if (-not (Get-Command bun -ErrorAction SilentlyContinue)) {
  throw "bun is required to build packages/agena-studio-web"
}

Push-Location $WebDir
try {
  & bun install
  if ($LASTEXITCODE -ne 0) {
    throw "bun install failed"
  }

  & bun run build
  if ($LASTEXITCODE -ne 0) {
    throw "bun run build failed"
  }
}
finally {
  Pop-Location
}
