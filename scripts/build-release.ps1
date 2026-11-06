<#
.SYNOPSIS
  Builds the release overlay_input.exe and copies it to a folder.

.DESCRIPTION
  1. Stops a running Overlay Input (a running exe can't be overwritten).
  2. cargo build --release
  3. Copies overlay_input.exe to -Out. Profiles, settings and logs are created next to it on first start.

.EXAMPLE
  .\scripts\build-release.ps1
  .\scripts\build-release.ps1 -Out C:\Tools\OverlayInput
#>
param(
    # Folder that gets overlay_input.exe (default: dist\OverlayInput in the repo).
    [string]$Out = (Join-Path $PSScriptRoot '..\dist\OverlayInput')
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }

function Invoke-Checked($what, [scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)" }
}

$running = Get-Process overlay_input -ErrorAction SilentlyContinue
if ($running) {
    Step 'Stopping the running Overlay Input'
    $running | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

Step 'Building overlay_input.exe (release, takes a minute or two)'
Invoke-Checked 'cargo build' { cargo build --release }

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Step "Copying to $Out"
Copy-Item -Force target\release\overlay_input.exe $Out
$exe = Join-Path $Out 'overlay_input.exe'
$size = [math]::Round((Get-Item $exe).Length / 1MB, 1)

Write-Host "`nDone: $exe ($size MB)" -ForegroundColor Green
if (Test-Path (Join-Path $Out 'profiles')) {
    Write-Host "Profiles: $(Join-Path $Out 'profiles') (kept as they are)"
}
else {
    Write-Host "Profiles: created in $(Join-Path $Out 'profiles') on first start, with the example profiles."
}
