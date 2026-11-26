# Legacy layout -> PBIR oracle (G13): Power BI Desktop converts each generated legacy project.
#
#   $d = "$env:TEMP\oracle-reports\layout"     # any work folder outside the repository
#   cargo xtask layout-oracle gen $d
#   bicraft run --file $d\cmds.json
#   cargo xtask layout-oracle install $d
#   if (-not $env:BICRAFT_LOCKS_DIR) { $env:BICRAFT_LOCKS_DIR = "$env:LOCALAPPDATA\bicraft\locks" }
#   mkdir $env:BICRAFT_LOCKS_DIR\desktop      # one Desktop at a time
#   powershell -File tools\layout-oracle\run.ps1 -Dir $d -Out $env:BICRAFT_TESTDATA\layout-oracle
#   rmdir $env:BICRAFT_LOCKS_DIR\desktop
#   $env:BICRAFT_TESTDATA=...; cargo test -p bicraft-engine --test layout_oracle -- --nocapture
#
# For each case, a copy of the legacy project is opened in Desktop; the Save button of the
# ribbon is invoked through UI Automation, Desktop asks whether to upgrade the report to PBIR
# (an HTML dialog), and the dialog's default button is clicked through its own DOM (MSHTML,
# WM_HTML_GETOBJECT). No keyboard input, no Desktop setting is changed. Desktop then writes
# `definition/`; the input and Desktop's output are copied to OUT\<case>\legacy and
# OUT\<case>\desktop. Only the Desktop instance this script started is closed.
param(
  [Parameter(Mandatory = $true)][string]$Dir,
  [Parameter(Mandatory = $true)][string]$Out,
  [string[]]$Cases = @('visuals', 'filters', 'pages', 'bookmarks', 'mobile', 'report', 'customvisual'),
  [string]$Work = (Join-Path $env:TEMP 'bicraft-layout-oracle-work'),
  [int]$OpenTimeout = 240,
  [int]$SaveTimeout = 120
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public class LoHtml {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern uint RegisterWindowMessage(string s);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, IntPtr w, IntPtr l, uint f, uint t, out IntPtr r);
  [DllImport("oleacc.dll", PreserveSig=false)] [return: MarshalAs(UnmanagedType.IDispatch)]
  public static extern object ObjectFromLresult(IntPtr l, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, IntPtr w);
}
"@
$AE = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$Raw = [System.Windows.Automation.TreeWalker]::RawViewWalker

function Top-Windows([int]$ProcId) {
  $AE::RootElement.FindAll($TS::Children, (New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $ProcId)))
}

function Find-Save([int]$ProcId) {
  foreach ($w in (Top-Windows $ProcId)) {
    $hits = $w.FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, 'save')))
    foreach ($h in $hits) { if ($h.Current.IsEnabled -and -not $h.Current.IsOffscreen) { return $h } }
  }
  return $null
}

# The upgrade dialog: a top-level window of the process whose title names PBIR, hosting an
# Internet Explorer_Server control.
function Find-UpgradeDialog([int]$ProcId) {
  foreach ($w in (Top-Windows $ProcId)) {
    $all = $w.FindAll($TS::Subtree, (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::Window)))
    foreach ($d in $all) {
      if ($d.Current.Name -and $d.Current.Name.Contains('(PBIR)')) { return $d }
    }
  }
  return $null
}

function Find-Ie($el) {
  if ($el.Current.ClassName -eq 'Internet Explorer_Server') { return $el }
  $k = $Raw.GetFirstChild($el)
  while ($k -ne $null) { $r = Find-Ie $k; if ($r) { return $r }; $k = $Raw.GetNextSibling($k) }
  return $null
}

# Clicks the dialog's default button (the upgrade).
function Click-Default($ie) {
  $msg = [LoHtml]::RegisterWindowMessage('WM_HTML_GETOBJECT')
  $r = [IntPtr]::Zero
  [LoHtml]::SendMessageTimeout([IntPtr]$ie.Current.NativeWindowHandle, $msg, [IntPtr]::Zero, [IntPtr]::Zero, 2, 5000, [ref]$r) | Out-Null
  if ($r -eq [IntPtr]::Zero) { return $false }
  $doc = [LoHtml]::ObjectFromLresult($r, [Guid]'626FC520-A41E-11CF-A731-00A0C9082637', [IntPtr]::Zero)
  $divs = $doc.getElementsByTagName('div')
  for ($i = 0; $i -lt $divs.length; $i++) {
    $d = $divs.item($i)
    if ($d.getAttribute('role') -eq 'button' -and ([string]$d.className).Contains('role-default')) { $d.click(); return $true }
  }
  return $false
}

function Wait-Stable([string]$Folder) {
  $last = -1
  for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 2
    $n = (Get-ChildItem $Folder -Recurse -File | Measure-Object -Property Length -Sum)
    $sig = "$($n.Count)/$($n.Sum)"
    if ($sig -eq $last) { return }
    $last = $sig
  }
}

# `powershell -File` passes a list as one comma-separated string.
$Cases = @($Cases | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
New-Item -ItemType Directory -Force $Work | Out-Null
New-Item -ItemType Directory -Force $Out | Out-Null
foreach ($case in $Cases) {
  $src = Join-Path $Dir $case
  $dst = Join-Path $Work $case
  if (Test-Path $dst) { Remove-Item -Recurse -Force $dst }
  Copy-Item -Recurse $src $dst
  $pbip = Join-Path $dst "$case.pbip"
  $report = Join-Path $dst "$case.Report"
  if (-not (Test-Path (Join-Path $report 'report.json'))) { "${case}: not a legacy project (run layout-oracle install)"; continue }
  $before = @(Get-Process PBIDesktop -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
  Start-Process -FilePath $pbip
  $procId = $null; $save = $null; $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt $OpenTimeout) {
    Start-Sleep -Seconds 5
    if (-not $procId) {
      $mine = Get-CimInstance Win32_Process -Filter "Name='PBIDesktop.exe'" | Where-Object { $before -notcontains $_.ProcessId -and $_.CommandLine -like "*$case.pbip*" } | Select-Object -First 1
      if ($mine) { $procId = [int]$mine.ProcessId }
      continue
    }
    $save = Find-Save $procId
    if ($save) { break }
  }
  if (-not $save) { "${case}: Desktop did not open (pid $procId)"; if ($procId) { Stop-Process -Id $procId -Force }; continue }
  Start-Sleep -Seconds 10   # let the report finish loading
  $save = Find-Save $procId
  $save.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
  $clicked = $false; $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt $SaveTimeout) {
    Start-Sleep -Seconds 3
    if (-not $clicked) {
      $dlg = Find-UpgradeDialog $procId
      if ($dlg) { $ie = Find-Ie $dlg; if ($ie) { $clicked = Click-Default $ie } }
      continue
    }
    if ((Test-Path (Join-Path $report 'definition\version.json')) -and -not (Test-Path (Join-Path $report 'report.json'))) { break }
  }
  if (-not (Test-Path (Join-Path $report 'definition\version.json'))) {
    "${case}: no PBIR written (upgrade clicked: $clicked)"
    Stop-Process -Id $procId -Force
    continue
  }
  Wait-Stable (Join-Path $report 'definition')
  Stop-Process -Id $procId -Force
  Start-Sleep -Seconds 3
  $o = Join-Path $Out $case
  if (Test-Path $o) { Remove-Item -Recurse -Force $o }
  New-Item -ItemType Directory -Force $o | Out-Null
  Copy-Item -Recurse $src (Join-Path $o 'legacy')
  New-Item -ItemType Directory -Force (Join-Path $o 'desktop') | Out-Null
  Copy-Item -Recurse (Join-Path $report 'definition') (Join-Path $o 'desktop\definition')
  Copy-Item (Join-Path $report 'definition.pbir') (Join-Path $o 'desktop\definition.pbir')
  if (Test-Path (Join-Path $report 'StaticResources')) { Copy-Item -Recurse (Join-Path $report 'StaticResources') (Join-Path $o 'desktop\StaticResources') }
  $files = (Get-ChildItem (Join-Path $o 'desktop\definition') -Recurse -File).Count
  "${case}: PBIR written by Desktop ($files files) -> $o"
}
Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
