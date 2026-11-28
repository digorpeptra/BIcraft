# Installs BIcraft for the current user: bicraft.exe (the desktop app) and bicraft-cli.exe (the
# CLI and MCP server).
#
#   powershell -ExecutionPolicy Bypass -File .\install.ps1 [-Destination DIR]
#
# Copies both from this folder to DIR (default %LOCALAPPDATA%\bicraft\bin). It does not change
# PATH, the registry or any MCP client configuration: it prints what to do instead.
param(
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'bicraft\bin')
)
$ErrorActionPreference = 'Stop'

foreach ($name in 'bicraft-cli.exe', 'bicraft.exe') {
    if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot $name) -PathType Leaf)) {
        Write-Error "$name not found next to install.ps1 ($PSScriptRoot)"
        exit 1
    }
}
New-Item -ItemType Directory -Force -Path $Destination | Out-Null

$cli = Join-Path $Destination 'bicraft-cli.exe'
try {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'bicraft-cli.exe') -Destination $cli -Force
} catch {
    Write-Error "cannot write $cli (is a bicraft-cli MCP server still running?): $_"
    exit 1
}
$version = & $cli --version
Write-Host "Installed $version"
Write-Host "  -> $cli (the CLI and MCP server)"

$app = Join-Path $Destination 'bicraft.exe'
try {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'bicraft.exe') -Destination $app -Force
    Write-Host "  -> $app (the desktop app; Start menu entry and Open with: bicraft.exe --register)"
} catch {
    Write-Error "cannot write $app (is the desktop app running?): $_"
    exit 1
}

$dir = (Resolve-Path -LiteralPath $Destination).Path.TrimEnd('\')
$onPath = $env:PATH -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ieq $dir }
if (-not $onPath) {
    Write-Host ''
    Write-Host "$dir is not on PATH. To add it for your user (new terminals pick it up):"
    Write-Host "  [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';$dir', 'User')"
}
Write-Host ''
Write-Host 'MCP clients: run `bicraft-cli setup --dry-run` to see the registration plan, then'
Write-Host '`bicraft-cli setup --client claude_code --yes` (or another client); see docs\mcp.md.'
