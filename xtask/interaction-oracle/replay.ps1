# Runs again, on the engine of the same Desktop instance, every distinct DAX query the trace
# recorded (interaction oracle, docs/interaction-oracle.md), and writes their results: per query,
# per EVALUATE, the columns (name, client type) and the rows (blank as null, dates as ISO text).
# A query the engine refuses is recorded as an error without the engine's message text (Microsoft
# text; it stays out of the repo). Read only: no change to the model.
# usage: replay.ps1 -Port 51234 -Trace trace.jsonl -Out results.json [-Amo DIR]
# -Amo DIR: the folder of the public AMO / ADOMD.NET client libraries; default $env:BICRAFT_AMO_DIR,
# else 'C:\Program Files\DAX Studio\bin' (DAX Studio ships them).
param([int]$Port, [string]$Trace, [string]$Out, [string]$Amo = $(if ($env:BICRAFT_AMO_DIR) { $env:BICRAFT_AMO_DIR } else { 'C:\Program Files\DAX Studio\bin' }))
$ErrorActionPreference = 'Stop'
try { Add-Type -Path (Join-Path $Amo 'Microsoft.AnalysisServices.AdomdClient.dll') } catch { }
$texts = New-Object System.Collections.Generic.List[string]
$seen = New-Object 'System.Collections.Generic.HashSet[string]'
foreach ($line in [IO.File]::ReadAllLines($Trace)) {
    if (-not $line.Trim()) { continue }
    $e = $line | ConvertFrom-Json
    if ($e.event -ne 'QueryBegin' -or $e.subclass -ne 'DAXQuery' -or -not $e.text) { continue }
    if ($seen.Add($e.text)) { $texts.Add($e.text) }
}
$conn = New-Object Microsoft.AnalysisServices.AdomdClient.AdomdConnection("Data Source=localhost:$Port")
$conn.Open()
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
$all = New-Object System.Collections.ArrayList
foreach ($t in $texts) {
    $entry = [ordered]@{ query = $t }
    try {
        $cmd = $conn.CreateCommand(); $cmd.CommandText = $t
        $r = $cmd.ExecuteReader()
        $sets = New-Object System.Collections.ArrayList
        try {
            do {
                $cols = @(for ($i = 0; $i -lt $r.FieldCount; $i++) { [ordered]@{ name = $r.GetName($i); type = (TypeName $r.GetFieldType($i)) } })
                $rows = New-Object System.Collections.ArrayList
                while ($r.Read()) {
                    $o = [ordered]@{}
                    for ($i = 0; $i -lt $r.FieldCount; $i++) { $o[$r.GetName($i)] = Cell $r.GetValue($i) }
                    [void]$rows.Add($o)
                }
                [void]$sets.Add([ordered]@{ columns = $cols; rows = $rows })
            } while ($r.NextResult())
        } finally { $r.Close() }
        $entry.results = $sets
    } catch {
        $entry.error = $true
    }
    [void]$all.Add($entry)
}
$conn.Close()
$json = [ordered]@{ version = 1; port = $Port; queries = $all } | ConvertTo-Json -Depth 20
[IO.File]::WriteAllText($Out, $json, (New-Object Text.UTF8Encoding($false)))
"replayed $($texts.Count) distinct queries -> $Out"
