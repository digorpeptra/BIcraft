<#
.SYNOPSIS
  Opens a workbook read-only in a new, invisible Excel and prints its data model and the
  results of the fixture's DAX queries, as Excel's own engine computes them.

.DESCRIPTION
  Uses Excel's public COM object model (Workbook.Model, ModelTables, ModelRelationships,
  ModelMeasures, Model.DataModelConnection.ModelConnection.ADOConnection) and closes only the
  Excel instance it started. The query results are the expectations of the engine test
  `a_workbook_excel_saved_opens_with_excels_results_and_saves_as_pbix`.

.PARAMETER Path
  Full path of the .xlsx to open.
#>
param([Parameter(Mandatory = $true)][string]$Path)
$ErrorActionPreference = 'Stop'
$Path = [System.IO.Path]::GetFullPath($Path)
Add-Type -Namespace Native -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint pid);
'@
$queries = @(
    'EVALUATE ROW("q", [Total quantity], "r", [Revenue], "c", [Customers])',
    'EVALUATE ADDCOLUMNS(VALUES(Customer[Country]), "r", [Revenue], "q", [Total quantity])',
    'EVALUATE ADDCOLUMNS(VALUES(Product[Category]), "r", [Revenue], "q", [Total quantity])'
)
$xl = New-Object -ComObject Excel.Application
[uint32]$excelPid = 0
[void][Native.Win]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$excelPid)
try {
    $xl.Visible = $false
    $xl.DisplayAlerts = $false
    "Excel $($xl.Version) build $($xl.Build)"
    $wb = $xl.Workbooks.Open($Path, 0, $true)
    $m = $wb.Model
    "tables: $($m.ModelTables.Count)"
    foreach ($t in $m.ModelTables) {
        $cols = ($t.ModelTableColumns | ForEach-Object { "$($_.Name):$($_.DataType)" }) -join ', '
        "  $($t.Name) rows=$($t.RecordCount) [$cols]"
    }
    "relationships: $($m.ModelRelationships.Count)"
    foreach ($r in $m.ModelRelationships) {
        "  $($r.ForeignKeyTable.Name)[$($r.ForeignKeyColumn.Name)] -> $($r.PrimaryKeyTable.Name)[$($r.PrimaryKeyColumn.Name)] active=$($r.Active)"
    }
    "measures: $($m.ModelMeasures.Count)"
    foreach ($x in $m.ModelMeasures) { "  $($x.AssociatedTable.Name)[$($x.Name)] = $($x.Formula)" }
    if ($m.ModelTables.Count -gt 0) {
        $m.Initialize()
        $cn = $m.DataModelConnection.ModelConnection.ADOConnection
        foreach ($q in $queries) {
            "## $q"
            $rs = $cn.Execute($q)
            while (-not $rs.EOF) {
                ($(for ($i = 0; $i -lt $rs.Fields.Count; $i++) { "$($rs.Fields.Item($i).Name)=$($rs.Fields.Item($i).Value)" }) -join '; ')
                $rs.MoveNext()
            }
            $rs.Close()
        }
    }
    $wb.Close($false)
}
finally {
    try { $xl.Quit() } catch {}
    [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl)
    [GC]::Collect(); [GC]::WaitForPendingFinalizers()
    Start-Sleep -Milliseconds 1500
    if ($excelPid -ne 0) {
        $p = Get-Process -Id $excelPid -ErrorAction SilentlyContinue
        if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $excelPid -Force }
    }
}
