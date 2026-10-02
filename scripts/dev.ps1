<#
.SYNOPSIS
  VibeRunner dev launcher — Windows.

.DESCRIPTION
  scripts\dev.ps1 start   # run `pnpm tauri dev` in the foreground
  scripts\dev.ps1 stop    # stop only what a previous `start` launched

  Mirrors scripts/dev.sh. The script exists so that Stop is reliable and
  scoped: `pnpm tauri dev` spawns vite, cargo, and the app itself, and
  Ctrl+C alone leaves cargo's children alive holding the dev port, so the
  next Start fails with "port already in use".

  `start` records the root PID *and its start time* in
  .codex\viberunner-dev.state. `stop` verifies both before signalling.
  The start time is the part that matters: a bare PID is not proof of
  identity because Windows recycles PIDs, and without the check a stale
  state file could eventually kill an unrelated process.

  PowerShell cannot exec-replace its own process the way a POSIX shell
  does, so this launcher stays alive as the recorded root and pnpm runs
  as its child. taskkill /T then covers the whole subtree from that
  root.

  `.codex\*` is gitignored (see .gitignore), so the state file is
  runtime-only.

  Note: paths here must use backslashes. `powershell.exe -File` rejects
  forward slashes outright ("Illegal characters in path"), which is why
  environment.toml spells the invocation as -File ".\scripts\dev.ps1".
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('start', 'stop')]
    [string]$Mode = 'start'
)

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
$State = Join-Path $Root '.codex\viberunner-dev.state'

function Read-State {
    if (-not (Test-Path -LiteralPath $State)) { return $null }
    $kv = @{}
    foreach ($line in Get-Content -LiteralPath $State) {
        if ($line -match '^(?<k>[A-Za-z]+)=(?<v>.*)$') { $kv[$Matches.k] = $Matches.v }
    }
    if (-not $kv.ContainsKey('pid') -or -not $kv.ContainsKey('started')) { return $null }
    return $kv
}

<#
  Returns the recorded PID only if it is still the exact process we
  started; $null when nothing of ours is left running.
#>
function Get-LivePid {
    $kv = Read-State
    if ($null -eq $kv) { return $null }

    $target = 0
    if (-not [int]::TryParse($kv['pid'], [ref]$target)) { return $null }

    $proc = Get-Process -Id $target -ErrorAction SilentlyContinue
    if ($null -eq $proc) { return $null }

    $now = $proc.StartTime.ToString('o')
    if ($now -ne $kv['started']) {
        Write-Host "note: pid $target now belongs to a different process; ignoring stale state"
        return $null
    }
    return $target
}

function Clear-State {
    Remove-Item -LiteralPath $State -Force -ErrorAction SilentlyContinue
}

function Start-Dev {
    $existing = Get-LivePid
    if ($null -ne $existing) {
        Write-Error "already running (pid $existing) - run 'stop' first"
        exit 1
    }

    if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
        Write-Error 'pnpm not found on PATH'
        exit 1
    }

    # Clear stale state so a later Stop can never target a recycled PID.
    Clear-State
    $stateDir = Split-Path -Parent $State
    if (-not (Test-Path -LiteralPath $stateDir)) {
        New-Item -ItemType Directory -Path $stateDir -Force | Out-Null
    }

    $self = Get-Process -Id $PID
    @(
        "pid=$PID"
        "started=$($self.StartTime.ToString('o'))"
    ) | Set-Content -LiteralPath $State -Encoding UTF8

    Push-Location $Root
    try {
        & pnpm tauri dev
        $code = $LASTEXITCODE
    } finally {
        Pop-Location
        # Reached only on a clean exit; a taskkill'd launcher leaves the
        # file behind, which Get-LivePid then correctly treats as stale.
        Clear-State
    }
    exit $code
}

function Stop-Dev {
    $target = Get-LivePid
    if ($null -eq $target) {
        if (Test-Path -LiteralPath $State) {
            Write-Host 'no live dev process recorded; clearing stale state'
        } else {
            Write-Host 'not running'
        }
        Clear-State
        exit 0
    }

    Write-Host "stopping VibeRunner dev tree (root pid $target)..."
    # /T covers every descendant of the recorded root, and nothing else.
    & taskkill /T /F /PID $target | Out-Null

    # Wait for the tree to actually go away so that
    # Start -> Stop -> Start is deterministic instead of racy.
    for ($i = 0; $i -lt 50; $i++) {
        if ($null -eq (Get-Process -Id $target -ErrorAction SilentlyContinue)) { break }
        Start-Sleep -Milliseconds 100
    }

    Clear-State
    Write-Host 'stopped'
}

switch ($Mode) {
    'start' { Start-Dev }
    'stop' { Stop-Dev }
}
