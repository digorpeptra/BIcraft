# Starts Power BI Desktop on one generated report with WebView2's DevTools port open (process
# environment only, as tools/render-oracle/capture.ps1) and prints the instance as JSON:
# {"pid": PBIDesktop, "engine_pid": msmdsrv, "port": engine port, "devtools": DevTools port}.
# Only the instance this script starts is ever closed (by record.ps1, with its pid).
# usage: launch.ps1 -File C:\...\interactions.pbix [-DevTools 9334] [-Wait 240] [-Out instance.json]
param([string]$File, [int]$DevTools = 9334, [int]$Wait = 240, [string]$Out = '')
$ErrorActionPreference = 'Stop'
$item = Get-Item -LiteralPath $File -ErrorAction SilentlyContinue
if (-not $item -or $item.Length -lt 1024) { "no such report file (or empty): $File"; exit 1 }
$File = $item.FullName
$before = @(Get-Process PBIDesktop -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
$started = Get-Date
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$DevTools --force-color-profile=srgb"
Start-Process -FilePath "$env:LOCALAPPDATA\Microsoft\WindowsApps\PBIDesktopStore.exe" -ArgumentList "`"$File`""
$leaf = [IO.Path]::GetFileName($File); $stem = [IO.Path]::GetFileNameWithoutExtension($File)
$mine = $null; $deadline = (Get-Date).AddSeconds($Wait)
while ((Get-Date) -lt $deadline) {
    Start-Sleep 5
    $mine = Get-CimInstance Win32_Process -Filter "Name='PBIDesktop.exe'" | Where-Object { $before -notcontains $_.ProcessId -and $_.CommandLine -like "*$leaf*" } | Select-Object -First 1
    if ($mine -and (Get-Process -Id $mine.ProcessId).MainWindowTitle -like "*$stem*") { break }
    $mine = $null
}
if (-not $mine) {
    Get-Process PBIDesktop -ErrorAction SilentlyContinue | Where-Object { $before -notcontains $_.Id -and $_.StartTime -ge $started } |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue; "closed $($_.Id)" }
    "Desktop did not start"; exit 1
}
# the engine: the msmdsrv child of this Desktop; its port file sits in its data folder
$engine = $null; $port = 0; $deadline = (Get-Date).AddSeconds(120)
while ((Get-Date) -lt $deadline -and $port -eq 0) {
    $engine = Get-CimInstance Win32_Process -Filter "Name='msmdsrv.exe' AND ParentProcessId=$($mine.ProcessId)" | Select-Object -First 1
    if ($engine -and $engine.CommandLine -match '-s\s+"([^"]+)"') {
        $pf = Join-Path $Matches[1] 'msmdsrv.port.txt'
        if (Test-Path -LiteralPath $pf) { $port = [int]((Get-Content -LiteralPath $pf -Encoding Unicode) -join '').Trim() }
    }
    if ($port -eq 0) { Start-Sleep 2 }
}
$o = [ordered]@{ pid = $mine.ProcessId; engine_pid = $(if ($engine) { $engine.ProcessId } else { 0 }); port = $port; devtools = $DevTools; file = $File; started = $started.ToString('o') }
$json = $o | ConvertTo-Json -Depth 20
if ($Out) { [IO.File]::WriteAllText($Out, $json, (New-Object Text.UTF8Encoding($false))) }
$json
