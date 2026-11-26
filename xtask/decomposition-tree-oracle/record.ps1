# Decomposition tree oracle recording (docs/render.md, "Decomposition tree"), Windows only: opens
# the generated report in a Desktop instance of its own (WebView2 DevTools port, process
# environment only), reads every page's tree with read.mjs, then closes that instance (only that
# one: its main window first, by its process id if it does not exit) and writes
# DIR\{instance.json, readings.json} (and DIR\shots\, page captures that stay local).
# usage: record.ps1 -Dir <work-dir> [-Settle 6000] [-DevTools 9352]
# Takes the Desktop lock `desktop` in $env:BICRAFT_LOCKS_DIR (default %LOCALAPPDATA%\bicraft\locks),
# waiting while it or the priority lock ($env:BICRAFT_PRIORITY_LOCK, default `priority`) exists.
param([string]$Dir, [int]$Settle = 6000, [int]$DevTools = 9352, [int]$LockWait = 1800)
$ErrorActionPreference = 'Stop'
$Dir = (Resolve-Path $Dir).Path
$pbix = Join-Path $Dir 'decomp.pbix'
$plan = Join-Path $Dir 'plan.json'
foreach ($f in $pbix, $plan) { if (-not (Test-Path $f)) { "missing $f (run oracle-gen decomp, bicraft run, patch first)"; exit 2 } }
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
$inst = $null
try {
    $instFile = Join-Path $Dir 'instance.json'
    & (Join-Path $PSScriptRoot '..\interaction-oracle\launch.ps1') -File $pbix -DevTools $DevTools -Out $instFile | Out-Null
    if (-not (Test-Path $instFile)) { 'Desktop did not start'; exit 1 }
    $inst = Get-Content -Raw $instFile | ConvertFrom-Json
    $pkg = Get-AppxPackage -Name 'Microsoft.MicrosoftPowerBIDesktop' -ErrorAction SilentlyContinue | Select-Object -First 1
    $meta = [ordered]@{ pid = $inst.pid; devtools = $inst.devtools; desktop = $(if ($pkg) { "$($pkg.Version)" } else { 'unknown' }); ui_culture = (Get-UICulture).Name; recorded = (Get-Date).ToString('o') }
    [IO.File]::WriteAllText($instFile, ($meta | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding($false)))
    Start-Sleep 20   # report view loaded
    node (Join-Path $PSScriptRoot 'read.mjs') $inst.devtools $plan (Join-Path $Dir 'readings.json') $Settle (Join-Path $Dir 'shots')
} finally {
    if ($inst -and $inst.pid) {
        $p = Get-Process -Id $inst.pid -ErrorAction SilentlyContinue
        if ($p -and $p.ProcessName -eq 'PBIDesktop') {
            $null = $p.CloseMainWindow()
            if (-not $p.WaitForExit(30000)) { Stop-Process -Id $inst.pid -Force; "forced $($inst.pid)" } else { "closed $($inst.pid)" }
        }
    }
    if ($locked) { Remove-Item (Join-Path $locks 'desktop') -Force -ErrorAction SilentlyContinue }
}
