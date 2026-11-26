<#
.SYNOPSIS
  Builds BIcraft's Excel-saved Power Pivot fixture with Excel's public COM object model.

.DESCRIPTION
  Starts its own Excel (TEMP and TMP set to -TempDir, so the data model backup records that
  folder instead of the user's temporary folder), writes three small tables of synthetic values
  on worksheets, adds each to the workbook's data model (Workbook.Connections.Add2 with
  CreateModelConnection), adds two relationships (Model.ModelRelationships.Add) and three
  measures (Model.ModelMeasures.Add with a format and a description), sets
  RemovePersonalInformation, saves the workbook as .xlsx and closes only the Excel it started.
  Every Excel call is documented in the Excel VBA reference on Microsoft Learn. Then run
  `cargo run -p bicraft-powerpivot --example scrub_workbook` on the result to replace the
  computer name the backup records (SOURCE.txt).

.PARAMETER OutPath
  Full path of the .xlsx to write (overwritten).

.PARAMETER TempDir
  A neutral folder (a short path named after no user) used as Excel's temporary folder.
#>
param(
    [Parameter(Mandatory = $true)][string]$OutPath,
    [Parameter(Mandatory = $true)][string]$TempDir
)

$ErrorActionPreference = 'Stop'
$OutPath = [System.IO.Path]::GetFullPath($OutPath)
if (Test-Path $OutPath) { Remove-Item $OutPath -Force }

Add-Type -Namespace Native -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint pid);
'@
# Looks a document up in the running object table without starting anything.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
namespace Native {
public static class Rot {
    [DllImport("ole32.dll")] static extern int GetRunningObjectTable(int reserved, out IRunningObjectTable rot);
    [DllImport("ole32.dll")] static extern int CreateBindCtx(int reserved, out IBindCtx ctx);
    public static object Find(string displayName) {
        IRunningObjectTable rot; IEnumMoniker e; IBindCtx ctx;
        if (GetRunningObjectTable(0, out rot) != 0 || CreateBindCtx(0, out ctx) != 0) return null;
        rot.EnumRunning(out e);
        IMoniker[] m = new IMoniker[1];
        while (e.Next(1, m, IntPtr.Zero) == 0) {
            string name; m[0].GetDisplayName(ctx, null, out name);
            if (string.Equals(name, displayName, StringComparison.OrdinalIgnoreCase)) {
                object o; rot.GetObject(m[0], out o); return o;
            }
        }
        return null;
    }
}
}
'@

function Set-Table($ws, [string]$name, [object[]]$header, [object[][]]$rows) {
    $r = $rows.Count + 1
    $c = $header.Count
    $data = New-Object 'object[,]' $r, $c
    for ($j = 0; $j -lt $c; $j++) { $data[0, $j] = $header[$j] }
    for ($i = 0; $i -lt $rows.Count; $i++) {
        for ($j = 0; $j -lt $c; $j++) { $data[($i + 1), $j] = $rows[$i][$j] }
    }
    $range = $ws.Range($ws.Cells(1, 1), $ws.Cells($r, $c))
    $range.Value2 = $data
    $lo = $ws.ListObjects.Add(1, $range, $null, 1) # xlSrcRange, xlYes (headers)
    $lo.Name = $name
    $ws.Name = $name
    return $lo
}

# Excel's data model engine keeps its working files under %TEMP% and records that folder in the
# backup it saves: start our own Excel with TEMP and TMP set to -TempDir, then attach to it.
New-Item -ItemType Directory -Force $TempDir | Out-Null
$TempDir = (Resolve-Path $TempDir).Path
if (Get-Process EXCEL -ErrorAction SilentlyContinue) { throw 'close every Excel window first: the script attaches to the Excel it starts' }
$env:TEMP = $TempDir
$env:TMP = $TempDir
$exe = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe').'(default)'
# An Excel started by hand is reached through the workbook it opens (its file moniker in the
# running object table): write an empty seed workbook, open it in our Excel, bind to it.
$seed = Join-Path $TempDir 'seed.xlsx'
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
if (Test-Path $seed) { Remove-Item $seed -Force }
$zip = [System.IO.Compression.ZipFile]::Open($seed, 'Create')
$seedParts = [ordered]@{
    '[Content_Types].xml' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>'
    '_rels/.rels' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>'
    'xl/workbook.xml' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>'
    'xl/_rels/workbook.xml.rels' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>'
    'xl/worksheets/sheet1.xml' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>'
}
foreach ($k in $seedParts.Keys) {
    $w = New-Object System.IO.StreamWriter($zip.CreateEntry($k).Open())
    $w.Write($seedParts[$k]); $w.Close()
}
$zip.Dispose()
$proc = Start-Process -FilePath $exe -ArgumentList '/x', "`"$seed`"" -PassThru -WindowStyle Minimized
$xl = $null
for ($i = 0; $i -lt 60 -and $null -eq $xl; $i++) {
    Start-Sleep -Milliseconds 500
    $rot = [Native.Rot]::Find($seed)
    if ($rot) { $xl = $rot.Application; $rot.Close($false) }
}
if ($null -eq $xl) { Stop-Process -Id $proc.Id -Force; throw 'the Excel started by this script did not register' }
[uint32]$excelPid = 0
[void][Native.Win]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$excelPid)
if ($excelPid -ne $proc.Id) { Stop-Process -Id $proc.Id -Force; throw "attached to Excel process $excelPid, not the one started ($($proc.Id))" }
try {
    $xl.Visible = $false
    $xl.DisplayAlerts = $false
    $xl.ScreenUpdating = $false
    $wb = $xl.Workbooks.Add()
    while ($wb.Worksheets.Count -lt 3) { [void]$wb.Worksheets.Add() }

    $product = Set-Table $wb.Worksheets.Item(1) 'Product' @('ProductKey', 'Product', 'Category', 'UnitPrice') @(
        @(1, 'Anvil', 'Tools', 12.5),
        @(2, 'Bolt', 'Tools', 0.25),
        @(3, 'Kite', 'Toys', 7.75),
        @(4, 'Yo-yo', 'Toys', 3),
        @(5, 'Lamp', 'Home', 19.99)
    )
    $customer = Set-Table $wb.Worksheets.Item(2) 'Customer' @('CustomerKey', 'Customer', 'Country') @(
        @(10, 'Alder Ltd', 'France'),
        @(20, 'Birch & Co', 'Belgium'),
        @(30, 'Cedar SA', 'France')
    )
    $d = [datetime]'2024-01-01'
    $salesRows = @()
    $qty = @(3, 10, 1, 4, 2, 7, 5, 1, 6, 2, 8, 9)
    $prod = @(1, 2, 3, 4, 5, 1, 2, 3, 2, 5, 4, 2)
    $cust = @(10, 20, 30, 10, 20, 30, 10, 10, 20, 30, 30, 20)
    for ($i = 0; $i -lt 12; $i++) {
        $salesRows += , @((101 + $i), $prod[$i], $cust[$i], $d.AddDays(7 * $i).ToOADate(), $qty[$i])
    }
    $salesWs = $wb.Worksheets.Item(3)
    $sales = Set-Table $salesWs 'Sales' @('SaleID', 'ProductKey', 'CustomerKey', 'OrderDate', 'Quantity') $salesRows
    $sales.ListColumns.Item('OrderDate').DataBodyRange.NumberFormat = 'yyyy-mm-dd'

    $wb.SaveAs($OutPath, 51) # xlOpenXMLWorkbook
    $book = $wb.Name
    foreach ($t in @('Product', 'Customer', 'Sales')) {
        # lCmdtype 7 = xlCmdExcel; CreateModelConnection = true; ImportRelationships = false
        [void]$wb.Connections.Add2("WorksheetConnection_$t", '', "WORKSHEET;$OutPath", "$book!$t", 7, $true, $false)
    }
    $model = $wb.Model
    $mt = $model.ModelTables
    $rels = $model.ModelRelationships
    [void]$rels.Add($mt.Item('Sales').ModelTableColumns.Item('ProductKey'), $mt.Item('Product').ModelTableColumns.Item('ProductKey'))
    [void]$rels.Add($mt.Item('Sales').ModelTableColumns.Item('CustomerKey'), $mt.Item('Customer').ModelTableColumns.Item('CustomerKey'))

    $fmtWhole = $model.ModelFormatWholeNumber(1)            # thousands separator
    $fmtDec = $model.ModelFormatDecimalNumber(1, 2)          # thousands separator, 2 decimals
    $measures = $model.ModelMeasures
    [void]$measures.Add('Total quantity', $mt.Item('Sales'), 'SUM(Sales[Quantity])', $fmtWhole, 'Units sold')
    [void]$measures.Add('Revenue', $mt.Item('Sales'), 'SUMX(Sales, Sales[Quantity] * RELATED(Product[UnitPrice]))', $fmtDec)
    [void]$measures.Add('Customers', $mt.Item('Customer'), 'COUNTROWS(Customer)', $model.ModelFormatGeneral)

    # No author, last editor or other personal information in the saved package.
    $wb.RemovePersonalInformation = $true
    $wb.Save()
    $wb.Close($false)
    "Excel $($xl.Version) build $($xl.Build) wrote $OutPath"
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

