Param()

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../..")
$WebDir = Join-Path $RepoRoot "packages/agena-web-ui"

if (-not (Get-Command bun -ErrorAction SilentlyContinue)) {
  throw "bun is required to build packages/agena-web-ui"
}

Push-Location $WebDir
try {
  & bun install --frozen-lockfile
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
