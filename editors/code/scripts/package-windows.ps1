$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not (Get-Command -Name bun -ErrorAction SilentlyContinue)) {
  Write-Error "bun is required to package the VSIX. Install bun and retry."
  exit 1
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location (Join-Path $scriptDir "..")

bun run package
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}
