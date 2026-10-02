<#
.SYNOPSIS
  VibeRunner validation + frontend build — Windows.

.DESCRIPTION
  This is the "Build" button: the fast, every-commit check. It runs the
  same gate AGENTS.md documents as the minimum bar (typecheck + Rust
  tests) and then produces the frontend bundle.

  It deliberately does NOT run `pnpm tauri build` — that is a 3-5 minute
  Rust release compile plus bundling. Use the "Package" action for a
  distributable.

  Usage:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File ".\scripts\build.ps1"
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $Root

function Invoke-Checked {
    param([string]$Exe, [string[]]$Arguments)
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Write-Error 'pnpm not found on PATH'
    exit 1
}

Write-Host '==> typecheck + Rust tests'
Invoke-Checked -Exe 'pnpm' -Arguments @('test')

Write-Host ''
Write-Host '==> frontend bundle'
Invoke-Checked -Exe 'pnpm' -Arguments @('build')

Write-Host ''
Write-Host 'ok: validation passed and dist\ is up to date'
