# Engine bench, reference answers (docs/benchmarks.md, "The DAX engine bench").
# Windows only, development aid. Opens ONE PBIX in a Power BI Desktop instance of its own, finds
# its Analysis Services engine, runs every query of queries.json once through ADOMD.NET and
# writes -Out (JSON: the Desktop and server versions, the fact's row count and each query's
# results), then closes only that instance (by its process id). Nothing is timed.
# Takes the Desktop lock `desktop` in $env:BICRAFT_LOCKS_DIR (default %LOCALAPPDATA%\bicraft\locks),
# waiting while it or the priority lock ($env:BICRAFT_PRIORITY_LOCK, default `priority`) exists.
# -ImageOut FILE: instead of the queries, a full refresh of the model by TOM in that engine (the
# M partitions read the CSV files again), then AMO ImageSave of the database to FILE, the
# DataModel of the bench PBIX (`cargo xtask engine-bench swap-datamodel`): data compressed by the
# engine itself.
# -Amo DIR: the folder of the public AMO / ADOMD.NET client libraries; default
# $env:BICRAFT_AMO_DIR, else 'C:\Program Files\DAX Studio\bin' (DAX Studio ships them).
# usage: desktop.ps1 -Pbix <data-dir>\r1m\bench.pbix -Out <data-dir>\r1m\desktop.json [-Only Q01,Q02]
param([string]$Pbix, [string]$Out, [string]$Only = '',
    [string]$Queries = (Join-Path $PSScriptRoot 'queries.json'),
    [string]$Amo = $(if ($env:BICRAFT_AMO_DIR) { $env:BICRAFT_AMO_DIR } else { 'C:\Program Files\DAX Studio\bin' }),
    [string]$ImageOut = '', [int]$LockWait = 3600, [int]$OpenWait = 1800, [int]$QueryTimeout = 600, [double]$MinFreeGB = 1.5)
$ErrorActionPreference = 'Stop'
$Pbix = (Resolve-Path $Pbix).Path
foreach ($n in 'Microsoft.AnalysisServices.Core', 'Microsoft.AnalysisServices', 'Microsoft.AnalysisServices.Tabular', 'Microsoft.AnalysisServices.AdomdClient') {
    try { Add-Type -Path (Join-Path $Amo "$n.dll") } catch { }
}

$spec = Get-Content -Raw -Encoding UTF8 $Queries | ConvertFrom-Json
$qs = @($spec.queries | Where-Object { -not $Only -or ($Only -split ',') -contains $_.id })

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

function FreeGB { (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB }
function TypeName([Type]$t) {
    switch ($t.FullName) {
        'System.Int64' { 'int64' } 'System.Int32' { 'int64' } 'System.Int16' { 'int64' }
        'System.Double' { 'double' } 'System.Single' { 'double' } 'System.Decimal' { 'decimal' }
        'System.DateTime' { 'dateTime' } 'System.String' { 'string' } 'System.Boolean' { 'boolean' }
        default { $null }
    }
}
function Cell($v) {
    if ($v -is [DBNull] -or $null -eq $v) { return $null }
    if ($v -is [DateTime]) { return $v.ToString('yyyy-MM-ddTHH:mm:ss', [Globalization.CultureInfo]::InvariantCulture) }
    if ($v -is [decimal]) { return [double]$v }
    if ($v -is [double] -and ([double]::IsInfinity($v) -or [double]::IsNaN($v))) { return "$v" }
    $v
}

$mine = $null; $srv = $null; $conn = $null
try {
    $before = @(Get-Process PBIDesktop -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
    Start-Process -FilePath "$env:LOCALAPPDATA\Microsoft\WindowsApps\PBIDesktopStore.exe" -ArgumentList "`"$Pbix`""
    $leaf = [IO.Path]::GetFileName($Pbix)
    $deadline = (Get-Date).AddSeconds($OpenWait)
    while ((Get-Date) -lt $deadline -and -not $mine) {
        Start-Sleep 2
        $mine = Get-CimInstance Win32_Process -Filter "Name='PBIDesktop.exe'" | Where-Object { $before -notcontains $_.ProcessId -and $_.CommandLine -like "*$leaf*" } | Select-Object -First 1
    }
    if (-not $mine) { throw 'Desktop did not start' }
    $pidDesktop = $mine.ProcessId
    # the engine (msmdsrv child of this Desktop) and the port file it writes in its workspace folder
    $port = 0; $engine = $null
    while ((Get-Date) -lt $deadline -and $port -eq 0) {
        $engine = Get-CimInstance Win32_Process -Filter "Name='msmdsrv.exe' AND ParentProcessId=$pidDesktop" | Select-Object -First 1
        if ($engine -and $engine.CommandLine -match '-s\s+"([^"]+)"') {
            $pf = Join-Path $Matches[1] 'msmdsrv.port.txt'
            if (Test-Path -LiteralPath $pf) { $port = [int]((Get-Content -LiteralPath $pf -Encoding Unicode) -join '').Trim() }
        }
        if ($port -eq 0) { Start-Sleep 1 }
    }
    if ($port -eq 0) { throw 'engine port not found' }
    # the model is loaded when the fact table answers
    $loaded = $false; $rows = $null
    while ((Get-Date) -lt $deadline -and -not $loaded) {
        if ((FreeGB) -lt $MinFreeGB) { throw "free memory below $MinFreeGB GB while Desktop loads the file" }
        try {
            if (-not $conn) { $conn = New-Object Microsoft.AnalysisServices.AdomdClient.AdomdConnection("Data Source=localhost:$port"); $conn.Open() }
            $c = $conn.CreateCommand(); $c.CommandText = 'EVALUATE ROW("n", COUNTROWS(Ventes))'; $c.CommandTimeout = 60
            $r = $c.ExecuteReader(); if ($r.Read()) { $rows = $r.GetValue(0) }; $r.Close(); $loaded = $true
        } catch { if ($conn) { try { $conn.Close() } catch { }; $conn = $null }; Start-Sleep 2 }
    }
    if (-not $loaded) { throw 'the model did not load' }
    "loaded $rows rows (pid $pidDesktop, port $port)"

    $srv = New-Object Microsoft.AnalysisServices.Tabular.Server
    $srv.Connect("Data Source=localhost:$port")
    $db = $srv.Databases[0]
    if ($ImageOut) {
        $db.Model.RequestRefresh([Microsoft.AnalysisServices.Tabular.RefreshType]::Full)
        [void]$db.Model.SaveChanges()
        $c = $conn.CreateCommand(); $c.CommandText = 'EVALUATE ROW("n", COUNTROWS(Ventes))'
        $r = $c.ExecuteReader(); if ($r.Read()) { $rows = $r.GetValue(0) }; $r.Close()
        $fs = [IO.File]::Create($ImageOut); try { $srv.ImageSave($db.ID, $fs) } finally { $fs.Close() }
        $o = [ordered]@{ rows = $rows; image_bytes = (Get-Item $ImageOut).Length }
        if ($Out) { [IO.File]::WriteAllText($Out, ($o | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false))) }
        "refreshed $rows rows, image $($o.image_bytes) bytes"
        return
    }

    function Results([string]$dax) {
        $c = $conn.CreateCommand(); $c.CommandText = $dax; $c.CommandTimeout = $QueryTimeout
        $r = $c.ExecuteReader()
        $sets = New-Object System.Collections.ArrayList
        try {
            do {
                $cols = @(for ($i = 0; $i -lt $r.FieldCount; $i++) { [ordered]@{ name = $r.GetName($i); type = (TypeName $r.GetFieldType($i)) } })
                $rowsOut = New-Object System.Collections.ArrayList
                $vals = New-Object object[] $r.FieldCount
                while ($r.Read()) { [void]$r.GetValues($vals); [void]$rowsOut.Add(@($vals | ForEach-Object { Cell $_ })) }
                [void]$sets.Add([ordered]@{ columns = $cols; rows = $rowsOut })
            } while ($r.NextResult())
        } finally { $r.Close() }
        $sets
    }

    $all = New-Object System.Collections.ArrayList
    foreach ($q in $qs) {
        if ((FreeGB) -lt $MinFreeGB) { throw "free memory below $MinFreeGB GB" }
        $e = [ordered]@{ id = $q.id; title = $q.title }
        try {
            $e.results = Results $q.dax
            "$($q.id) answered"
        } catch {
            $e.error = $true   # the engine's message is Microsoft text: kept out of the file
            $msg = $_.Exception; while ($msg.InnerException) { $msg = $msg.InnerException }
            "  $($q.id): engine error ($($msg.GetType().Name))"
        }
        [void]$all.Add($e)
    }
    $pkg = Get-AppxPackage -Name 'Microsoft.MicrosoftPowerBIDesktop' -ErrorAction SilentlyContinue | Select-Object -First 1
    $o = [ordered]@{
        engine = 'desktop'; desktop = $(if ($pkg) { "$($pkg.Version)" } else { 'unknown' }); server_version = $srv.Version
        pbix = $Pbix; rows = $rows; recorded = (Get-Date).ToString('o'); queries = $all
    }
    [IO.File]::WriteAllText($Out, ($o | ConvertTo-Json -Depth 30), (New-Object Text.UTF8Encoding($false)))
    "-> $Out"
} finally {
    if ($srv) { try { $srv.Disconnect() } catch { } }
    if ($conn) { try { $conn.Close() } catch { } }
    if ($mine) {
        $p = Get-Process -Id $mine.ProcessId -ErrorAction SilentlyContinue
        if ($p -and $p.ProcessName -eq 'PBIDesktop') { Stop-Process -Id $mine.ProcessId -Force; "closed $($mine.ProcessId)" }
    }
    Remove-Item (Join-Path $locks 'desktop') -Force -ErrorAction SilentlyContinue
}
