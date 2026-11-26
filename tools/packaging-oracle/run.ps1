# Packaging oracle: opens each variant of `cargo xtask pkg-oracle mutate` in Power BI Desktop and
# records whether Desktop accepts it (docs/formats/packaging-verification.md).
#
# usage ($w: any work folder): run.ps1 -Cases $w\cases.json -Out $w\results.json [-Only p00,v02] [-Page Probe] [-Value 742]
#
# Accepted = the report view lists the page `-Page` and its canvas shows `-Value` (a card bound to
# a measure, so the model loaded in Desktop's engine and answered a query). Refused = the page
# never appears (Desktop falls back to an empty report; error dialogs are recorded by name).
# "partial" = the page appears but the value does not (report read, model not loaded).
#
# Black-box only: Desktop is started on the file through its app execution alias, observed
# through UI Automation (window names) and its WebView2 DevTools port (page text), and closed.
# Takes the Desktop lock `desktop` in $env:BICRAFT_LOCKS_DIR (default %LOCALAPPDATA%\bicraft\locks)
# per case (one atomic mkdir; waits while it or the priority lock $env:BICRAFT_PRIORITY_LOCK,
# default `priority`, exists) and only closes the Desktop process tree it started.
param([string]$Cases, [string]$Out, [string[]]$Only = @(), [string]$Page = 'Probe', [string]$Value = '742',
      [int]$Port = 9361, [int]$Wait = 150, [int]$Cooldown = 90)
$ErrorActionPreference = 'Stop'
$locks = if ($env:BICRAFT_LOCKS_DIR) { $env:BICRAFT_LOCKS_DIR } else { Join-Path $env:LOCALAPPDATA 'bicraft\locks' }
$priorityLock = if ($env:BICRAFT_PRIORITY_LOCK) { $env:BICRAFT_PRIORITY_LOCK } else { 'priority' }
New-Item -ItemType Directory -Force $locks | Out-Null
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$list = Get-Content $Cases -Raw -Encoding UTF8 | ConvertFrom-Json
$results = [ordered]@{}
if (Test-Path $Out) { (Get-Content $Out -Raw -Encoding UTF8 | ConvertFrom-Json).PSObject.Properties | ForEach-Object { $results[$_.Name] = $_.Value } }

function Window-Names([int]$procId) {
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $procId)
  $wins = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
  $names = @()
  foreach ($w in $wins) {
    $names += $w.Current.Name
    # the main window's tree (WebView2 report) is large: only dialogs are searched for texts
    if ($w.Current.NativeWindowHandle -eq [int](Get-Process -Id $procId).MainWindowHandle) { continue }
    $texts = $w.FindAll([System.Windows.Automation.TreeScope]::Descendants,
      (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)))
    foreach ($t in $texts) { if ($t.Current.Name -and $t.Current.Name.Length -lt 400) { $names += "  text: " + $t.Current.Name } }
  }
  $names
}

foreach ($c in $list) {
  if ($Only.Count -gt 0 -and $Only -notcontains $c.id) { continue }
  if (-not (Test-Path -LiteralPath $c.path)) { "$($c.id): missing $($c.path)"; continue }
  # one Desktop at a time machine-wide; none while a benchmark asks for a quiet machine
  while ($true) {
    if (-not (Test-Path (Join-Path $locks $priorityLock)) -and -not (Test-Path (Join-Path $locks 'bench-quiet'))) {
      try { New-Item -ItemType Directory -Path (Join-Path $locks 'desktop') -ErrorAction Stop | Out-Null; break } catch { }
    }
    Start-Sleep 30
  }
  $mine = $null
  try {
    $pids = @(Get-Process PBIDesktop -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
    Start-Process -FilePath "$env:LOCALAPPDATA\Microsoft\WindowsApps\PBIDesktopStore.exe" -ArgumentList "`"$($c.path)`""
    $leaf = [IO.Path]::GetFileName($c.path)
    $verdict = 'refuse'; $probe = $null; $title = ''; $dialogs = @()
    $deadline = (Get-Date).AddSeconds($Wait)
    while ((Get-Date) -lt $deadline) {
      Start-Sleep 5
      if (-not $mine) {
        $mine = Get-CimInstance Win32_Process -Filter "Name='PBIDesktop.exe'" | Where-Object { $pids -notcontains $_.ProcessId -and $_.CommandLine -like "*$leaf*" } | Select-Object -First 1
        if (-not $mine) { continue }
      }
      $p = Get-Process -Id $mine.ProcessId -ErrorAction SilentlyContinue
      if (-not $p) { break }
      $title = $p.MainWindowTitle
      $probe = node (Join-Path $PSScriptRoot 'probe.mjs') $Port | ConvertFrom-Json
      if ($probe.tabs -and ($probe.tabs -contains $Page)) {
        $verdict = 'partial'
        if ($probe.text -match [regex]::Escape($Value)) { $verdict = 'accept'; break }
      }
    }
    if ($mine) { $dialogs = @(Window-Names $mine.ProcessId) }
    $results[$c.id] = [ordered]@{ fact = $c.fact; expect = $c.expect; verdict = $verdict; title = $title;
      tabs = $probe.tabs; windows = $dialogs; when = (Get-Date).ToString('s') }
    "$($c.id): $verdict (expected $($c.expect)) - $($c.fact)"
  } finally {
    if ($mine) { & taskkill.exe /T /F /PID $mine.ProcessId 2>&1 | Out-Null }
    else {
      # a late start of this launch: only a new Desktop opened on this very file
      Get-CimInstance Win32_Process -Filter "Name='PBIDesktop.exe'" | Where-Object { $pids -notcontains $_.ProcessId -and $_.CommandLine -like "*$leaf*" } |
        ForEach-Object { & taskkill.exe /T /F /PID $_.ProcessId 2>&1 | Out-Null }
    }
    Remove-Item -LiteralPath (Join-Path $locks 'desktop') -Force
    $results | ConvertTo-Json -Depth 5 | Set-Content -Path $Out -Encoding UTF8
  }
  # let the other agents waiting for Desktop take the lock before the next case
  Start-Sleep $Cooldown
}
