# Captures Power BI Desktop's own rendering of every listed page, without the screen: Desktop is
# started with WebView2's DevTools port open (process environment only) and sRGB output, then
# capture.mjs switches pages and takes Page.captureScreenshot of the report canvas.
# usage ($w: any work folder): capture.ps1 -File $w\synth.pbix -Out $w\desktop -Pages Cards,Columns,...
#        [-Width 1280] [-Port 9333]
# Take the Desktop lock first (`desktop` in $env:BICRAFT_LOCKS_DIR, and wait while the priority
# lock $env:BICRAFT_PRIORITY_LOCK exists there); only the Desktop instance this script starts is
# closed.
param([string]$File, [string]$Out, [string[]]$Pages, [int]$Width = 1280, [int]$Port = 9333, [int]$Wait = 200)
# never launch Desktop on a missing or partly written file (it would leave an error dialog open)
$item = Get-Item -LiteralPath $File -ErrorAction SilentlyContinue
if (-not $item -or $item.Length -lt 1024) { "no such report file (or empty): $File"; exit 1 }
$File = $item.FullName
$pids = @(Get-Process PBIDesktop -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
$started = Get-Date
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port --force-color-profile=srgb"
# the app execution alias passes this environment on (opening the .pbix through the shell does not)
Start-Process -FilePath "$env:LOCALAPPDATA\Microsoft\WindowsApps\PBIDesktopStore.exe" -ArgumentList "`"$File`""
$leaf = [IO.Path]::GetFileName($File); $stem = [IO.Path]::GetFileNameWithoutExtension($File)
$mine = $null; $deadline = (Get-Date).AddSeconds($Wait)
while ((Get-Date) -lt $deadline) {
  Start-Sleep 5
  $mine = Get-CimInstance Win32_Process -Filter "Name='PBIDesktop.exe'" | Where-Object { $pids -notcontains $_.ProcessId -and $_.CommandLine -like "*$leaf*" } | Select-Object -First 1
  if ($mine -and (Get-Process -Id $mine.ProcessId).MainWindowTitle -like "*$stem*") { break }
}
if (-not $mine) {
  # close whatever this launch started (any Desktop that was not running before, started since)
  Get-Process PBIDesktop -ErrorAction SilentlyContinue | Where-Object { $pids -notcontains $_.Id -and $_.StartTime -ge $started } |
    ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue; "closed $($_.Id)" }
  "Desktop did not start"; exit 1
}
try {
  Start-Sleep 25   # report view loaded and rendered
  node (Join-Path $PSScriptRoot "capture.mjs") $Port $Out $Width @Pages
} finally {
  Stop-Process -Id $mine.ProcessId -Force -ErrorAction SilentlyContinue
  "closed $($mine.ProcessId)"
}
