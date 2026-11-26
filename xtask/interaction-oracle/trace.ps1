# Records the queries Power BI Desktop sends to its local Analysis Services engine (interaction
# oracle, docs/interaction-oracle.md): an AMO server trace of QueryBegin / QueryEnd on the engine
# of one Desktop instance, one JSON line per event in -Out, until -StopFile appears.
# Windows only, development aid. The trace is a server object: it is created with a unique name and
# dropped on exit; it only reads (no change to the model).
# usage: trace.ps1 -Port 51234 -Out C:\...\trace.jsonl -StopFile C:\...\stop [-Amo DIR]
# -Amo DIR: the folder of the public AMO / ADOMD.NET client libraries; default $env:BICRAFT_AMO_DIR,
# else 'C:\Program Files\DAX Studio\bin' (DAX Studio ships them).
param([int]$Port, [string]$Out, [string]$StopFile, [string]$Amo = $(if ($env:BICRAFT_AMO_DIR) { $env:BICRAFT_AMO_DIR } else { 'C:\Program Files\DAX Studio\bin' }), [int]$MaxSeconds = 3600)
$ErrorActionPreference = 'Stop'
foreach ($n in 'Microsoft.AnalysisServices.Core', 'Microsoft.AnalysisServices.Tabular') {
    try { Add-Type -Path (Join-Path $Amo "$n.dll") } catch { }
}
$refs = @((Join-Path $Amo 'Microsoft.AnalysisServices.Core.dll'), (Join-Path $Amo 'Microsoft.AnalysisServices.Tabular.dll'))
if (-not ('OracleTraceSink' -as [type])) {
    Add-Type -ReferencedAssemblies $refs -TypeDefinition @'
using System;
using System.IO;
using System.Text;
using Microsoft.AnalysisServices;
public class OracleTraceSink {
    private readonly StreamWriter w;
    private readonly object gate = new object();
    public int Count;
    public OracleTraceSink(string path) {
        w = new StreamWriter(path, false, new UTF8Encoding(false));
        w.AutoFlush = true;
    }
    public void Attach(Microsoft.AnalysisServices.Tabular.Trace t) {
        t.OnEvent += new Microsoft.AnalysisServices.Tabular.TraceEventHandler(On);
    }
    private static void Str(StringBuilder sb, string s) {
        sb.Append('"');
        foreach (char c in s ?? "") {
            switch (c) {
                case '"': sb.Append("\\\""); break;
                case '\\': sb.Append("\\\\"); break;
                case '\n': sb.Append("\\n"); break;
                case '\r': sb.Append("\\r"); break;
                case '\t': sb.Append("\\t"); break;
                default:
                    if (c < ' ') sb.AppendFormat("\\u{0:x4}", (int)c); else sb.Append(c);
                    break;
            }
        }
        sb.Append('"');
    }
    private static void Field(StringBuilder sb, string name, Func<string> get) {
        string v;
        try { v = get(); } catch { return; }
        if (v == null) return;
        sb.Append(','); Str(sb, name); sb.Append(':'); Str(sb, v);
    }
    private void On(object sender, Microsoft.AnalysisServices.Tabular.TraceEventArgs e) {
        var sb = new StringBuilder();
        sb.Append("{\"event\":"); Str(sb, e.EventClass.ToString());
        Field(sb, "subclass", () => e.EventSubclass.ToString());
        Field(sb, "text", () => e.TextData);
        Field(sb, "start", () => e.StartTime.ToString("o"));
        Field(sb, "current", () => e.CurrentTime.ToString("o"));
        Field(sb, "session", () => e.SessionID);
        Field(sb, "connection", () => e[TraceColumn.ConnectionID]);
        Field(sb, "database", () => e.DatabaseName);
        Field(sb, "properties", () => e[TraceColumn.RequestProperties]);
        Field(sb, "parameters", () => e[TraceColumn.RequestParameters]);
        Field(sb, "error", () => e[TraceColumn.Error]);
        Field(sb, "application", () => e[TraceColumn.ApplicationName]);
        sb.Append(",\"logged\":"); Str(sb, DateTime.Now.ToString("o"));
        sb.Append('}');
        lock (gate) { w.WriteLine(sb.ToString()); Count++; }
    }
    public void Close() { lock (gate) { w.Flush(); w.Dispose(); } }
}
'@
}
$srv = New-Object Microsoft.AnalysisServices.Tabular.Server
$srv.Connect("Data Source=localhost:$Port")
$name = 'bicraft-interaction-oracle-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$tr = $srv.Traces.Add($name)
$wanted = @{
    'QueryBegin' = @('TextData', 'StartTime', 'CurrentTime', 'SessionID', 'ConnectionID', 'DatabaseName', 'RequestProperties', 'RequestParameters', 'EventSubclass', 'ApplicationName')
    'QueryEnd'   = @('TextData', 'StartTime', 'CurrentTime', 'SessionID', 'ConnectionID', 'DatabaseName', 'EventSubclass', 'Error', 'ApplicationName')
}
foreach ($ev in $wanted.Keys) {
    $te = $tr.Events.Add([Microsoft.AnalysisServices.TraceEventClass]::$ev)
    foreach ($c in $wanted[$ev]) {
        try { [void]$te.Columns.Add([Microsoft.AnalysisServices.TraceColumn]::$c) } catch { }
    }
}
$tr.Update()
$sink = New-Object OracleTraceSink($Out)
$sink.Attach($tr)
$tr.Start()
"trace $name started on port $Port"
$deadline = (Get-Date).AddSeconds($MaxSeconds)
try {
    while (-not (Test-Path -LiteralPath $StopFile) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300 }
    Start-Sleep -Milliseconds 800   # last events of the final state
} finally {
    try { $tr.Stop() } catch { }
    try { $tr.Drop() } catch { }
    $sink.Close()
    $srv.Disconnect()
    "trace $name stopped: $($sink.Count) events"
}
