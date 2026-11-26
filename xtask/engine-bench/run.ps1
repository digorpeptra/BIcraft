# Engine bench, one size end to end (docs/benchmarks.md, "The DAX engine bench").
# Windows only. For -Rows N, in -Data\r<N>:
#   seed     CSV files of 10 000 sales lines, `bicraft run` builds seed.pbix (M partitions over the
#            CSV files), then the CSV files of N lines replace them (same paths)
#   refresh  Desktop opens seed.pbix, TOM full refresh (Desktop's engine reads the N lines),
#            AMO ImageSave -> image.xp9; swap-datamodel -> bench.pbix (data compressed by the engine)
#   own      (instead of seed + refresh, no Desktop) CSV files of N lines, `bicraft run` builds
#            bench.pbix with the data encoded by bicraft itself: for bicraft's timings alone
#   desktop  desktop.ps1 on bench.pbix -> desktop.json (the reference answers, untimed)
#   bicraft engine-bench bicraft on bench.pbix -> bicraft.json (bicraft's timings and answers)
#   compare  bicraft's answers against the reference -> answers.md, answers.json
# usage: run.ps1 -Rows 1000000 [-Steps seed,refresh,desktop,bicraft,compare] [-Data <data-dir>]
#        [-Target <cargo-target-dir>] [-Warm 5] [-Fresh 1] [-MinFreeGB 0.8] [-TimeoutMs 120000]
#        [-Variant standard|bigdim|skew]  (a variant's folder is r<N>-<variant>, its queries queries-<variant>.json;
#        bigdim and skew have no bench report, so no aggregations)
# (bicraft: a query over -TimeoutMs is reported as refused at that budget; over 10 s it is repeated once)
param([long]$Rows, [string[]]$Steps = @('seed', 'refresh', 'desktop', 'bicraft', 'compare'),
    [string]$Data = $(if ($env:BICRAFT_BENCH_DATA) { $env:BICRAFT_BENCH_DATA } else { Join-Path $env:TEMP 'bicraft-bench-data' }),
    [string]$Target = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '..\..\target' }),
    [int]$Warm = 5, [int]$Fresh = 1, [double]$MinFreeGB = 0.8, [string]$Only = '', [int]$TimeoutMs = 120000, [string]$Variant = 'standard')
# native tools write progress to stderr: PowerShell 5.1 would turn it into errors under 'Stop'
$ErrorActionPreference = 'Continue'
$xtask = Join-Path $Target 'release\xtask.exe'
$bicraft = Join-Path $Target 'release\bicraft.exe'
function Label([long]$n) { if ($n % 1000000 -eq 0) { "r$($n / 1000000)m" } elseif ($n % 1000 -eq 0) { "r$($n / 1000)k" } else { "r$n" } }
$label = Label $Rows
if ($Variant -ne 'standard') { $label = "$label-$Variant" }
$dir = Join-Path $Data $label
$queries = if ($Variant -eq 'standard') { Join-Path $PSScriptRoot 'queries.json' } else { Join-Path $PSScriptRoot "queries-$Variant.json" }
# a variant's fact is in one file (one partition) per 4 M lines: the seed has the layout of N lines
$parts = [math]::Max(1, [math]::Ceiling($Rows / 4000000))
$partsArg = if ($Variant -eq 'standard') { @() } else { @('--parts', $parts) }
New-Item -ItemType Directory -Force $dir | Out-Null
$desktop = Join-Path $PSScriptRoot 'desktop.ps1'
foreach ($step in $Steps) {
    switch ($step) {
        'seed' {
            & $xtask engine-bench gen --rows 10000 --out $dir --variant $Variant @partsArg; if ($LASTEXITCODE) { throw 'gen (seed) failed' }
            & $bicraft run --file (Join-Path $dir 'build.json') > (Join-Path $dir 'build.out'); if ($LASTEXITCODE) { throw 'bicraft build failed' }
            Move-Item -Force (Join-Path $dir 'bench.pbix') (Join-Path $dir 'seed.pbix')
            & $xtask engine-bench gen --rows $Rows --out $dir --variant $Variant @partsArg; if ($LASTEXITCODE) { throw 'gen failed' }
        }
        'own' {
            & $xtask engine-bench gen --rows $Rows --out $dir --variant $Variant; if ($LASTEXITCODE) { throw 'gen failed' }
            & $bicraft run --file (Join-Path $dir 'build.json') > (Join-Path $dir 'build.out'); if ($LASTEXITCODE) { throw 'bicraft build failed' }
        }
        'refresh' {
            & powershell -NoProfile -ExecutionPolicy Bypass -File $desktop -Pbix (Join-Path $dir 'seed.pbix') -ImageOut (Join-Path $dir 'image.xp9') -Out (Join-Path $dir 'refresh.json') -MinFreeGB $MinFreeGB
            if ($LASTEXITCODE) { throw 'Desktop refresh failed' }
            & $xtask engine-bench swap-datamodel --pbix (Join-Path $dir 'seed.pbix') --image (Join-Path $dir 'image.xp9') --out (Join-Path $dir 'bench.pbix')
            if ($LASTEXITCODE) { throw 'swap-datamodel failed' }
        }
        'desktop' {
            $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $desktop, '-Pbix', (Join-Path $dir 'bench.pbix'), '-Out', (Join-Path $dir 'desktop.json'), '-MinFreeGB', $MinFreeGB, '-Queries', $queries)
            if ($Only) { $a += @('-Only', $Only) }
            & powershell @a
            if ($LASTEXITCODE) { throw 'Desktop reference answers failed' }
        }
        'bicraft' {
            $a = @('engine-bench', 'bicraft', '--bin', $bicraft, '--pbix', (Join-Path $dir 'bench.pbix'), '--out', (Join-Path $dir 'bicraft.json'), '--warm', $Warm, '--fresh', $Fresh, '--timeout-ms', $TimeoutMs, '--variant', $Variant)
            if ($Only) { $a += @('--only', $Only) }
            & $xtask @a
            if ($LASTEXITCODE) { throw 'bicraft measures failed' }
        }
        'compare' {
            & $xtask engine-bench compare --desktop (Join-Path $dir 'desktop.json') --bicraft (Join-Path $dir 'bicraft.json') --markdown (Join-Path $dir 'answers.md') --json (Join-Path $dir 'answers.json')
        }
        default { throw "unknown step $step" }
    }
    "[$step] done"
}
