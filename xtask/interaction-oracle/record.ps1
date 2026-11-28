# Interaction oracle recording (docs/interaction-oracle.md), Windows only: opens the generated
# report in a Desktop instance of its own, traces the queries its engine receives, plays the plan
# through the DevTools protocol, replays every traced query on the same engine, then closes that
# instance (only that one) and writes DIR\raw\{instance.json, trace.jsonl, readings.json,
# results.json}.
# usage: record.ps1 -Dir <work-dir> [-Settle 6000] [-Only I19,I20] [-Raw raw2]
# (several recordings, DIR\raw*, are merged by collect: a later one replaces the states it repeats)
# Takes the Desktop lock `desktop` in $env:BICRAFT_LOCKS_DIR (default %LOCALAPPDATA%\bicraft\locks),
# waiting while it or the priority lock ($env:BICRAFT_PRIORITY_LOCK, default `priority`) exists.
param([string]$Dir, [int]$Settle = 6000, [string]$Only = '', [int]$DevTools = 9334, [int]$LockWait = 1800, [string]$Raw = 'raw')
$ErrorActionPreference = 'Stop'
$Dir = (Resolve-Path $Dir).Path
$raw = Join-Path $Dir $Raw
New-Item -ItemType Directory -Force $raw | Out-Null
$pbix = Join-Path $Dir 'interactions.pbix'
$plan = Join-Path $Dir 'plan.json'
foreach ($f in $pbix, $plan) { if (-not (Test-Path $f)) { "missing $f (run oracle-gen interactions, bicraft-cli run, patch first)"; exit 2 } }

$locks = if ($env:BICRAFT_LOCKS_DIR) { $env:BICRAFT_LOCKS_DIR } else { Join-Path $env:LOCALAPPDATA 'bicraft\locks' }
$priorityLock = if ($env:BICRAFT_PRIORITY_LOCK) { $env:BICRAFT_PRIORITY_LOCK } else { 'priority' }
New-Item -ItemType Directory -Force $locks | Out-Null
$deadline = (Get-Date).AddSeconds($LockWait); $locked = $false
while ((Get-Date) -lt $deadline) {
    if (-not (Test-Path (Join-Path $locks $priorityLock))) {
        try { New-Item -ItemType Directory (Join-Path $locks 'desktop') -ErrorAction Stop | Out-Null; $locked = $true; break } catch { }
    }
    Start-Sleep 10
}
if (-not $locked) { 'Desktop lock busy'; exit 3 }
$inst = $null; $traceProc = $null
try {
    $instFile = Join-Path $raw 'instance.json'
    & (Join-Path $PSScriptRoot 'launch.ps1') -File $pbix -DevTools $DevTools -Out $instFile | Out-Null
    if (-not (Test-Path $instFile)) { 'Desktop did not start'; exit 1 }
    $inst = Get-Content -Raw $instFile | ConvertFrom-Json
    if ($inst.port -eq 0) { 'engine port not found'; exit 1 }
    $pkg = Get-AppxPackage -Name 'Microsoft.MicrosoftPowerBIDesktop' -ErrorAction SilentlyContinue | Select-Object -First 1
    $meta = [ordered]@{ pid = $inst.pid; port = $inst.port; devtools = $inst.devtools; desktop = $(if ($pkg) { "$($pkg.Version)" } else { 'unknown' }); ui_culture = (Get-UICulture).Name; recorded = (Get-Date).ToString('o') }
    [IO.File]::WriteAllText($instFile, ($meta | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding($false)))

    $trace = Join-Path $raw 'trace.jsonl'; $stop = Join-Path $raw 'trace.stop'
    Remove-Item $stop -ErrorAction SilentlyContinue
    $traceProc = Start-Process -FilePath powershell -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'trace.ps1'), '-Port', $inst.port, '-Out', $trace, '-StopFile', $stop)
    $t0 = Get-Date
    while (-not (Test-Path $trace) -and ((Get-Date) - $t0).TotalSeconds -lt 60) { Start-Sleep 1 }
    Start-Sleep 3
    if ($Only) { $env:PBC_ORACLE_ONLY = $Only } else { Remove-Item Env:PBC_ORACLE_ONLY -ErrorAction SilentlyContinue }
    node (Join-Path $PSScriptRoot 'drive.mjs') $inst.devtools $plan (Join-Path $raw 'readings.json') $Settle
    Start-Sleep 5   # the server sends trace events in batches
    New-Item -ItemType File $stop -Force | Out-Null
    $traceProc.WaitForExit(60000) | Out-Null
    & (Join-Path $PSScriptRoot 'replay.ps1') -Port $inst.port -Trace $trace -Out (Join-Path $raw 'results.json')
} finally {
    if ($traceProc -and -not $traceProc.HasExited) { Stop-Process -Id $traceProc.Id -Force -ErrorAction SilentlyContinue }
    if ($inst -and $inst.pid) {
        $p = Get-Process -Id $inst.pid -ErrorAction SilentlyContinue
        if ($p -and $p.ProcessName -eq 'PBIDesktop') { Stop-Process -Id $inst.pid -Force; "closed $($inst.pid)" }
    }
    Remove-Item (Join-Path $locks 'desktop') -Force -ErrorAction SilentlyContinue
}
