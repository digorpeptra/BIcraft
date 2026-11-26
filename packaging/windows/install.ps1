# Installs bicraft.exe (and bicraft-desktop.exe when present) for the current user.
#
#   powershell -ExecutionPolicy Bypass -File .\install.ps1 [-Destination DIR]
#
# Copies bicraft.exe from this folder to DIR (default %LOCALAPPDATA%\bicraft\bin). It does not
# change PATH, the registry or any MCP client configuration: it prints what to do instead.
param(
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'bicraft\bin')
)
$ErrorActionPreference = 'Stop'

$source = Join-Path $PSScriptRoot 'bicraft.exe'
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    Write-Error "bicraft.exe not found next to install.ps1 ($source)"
    exit 1
}
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$target = Join-Path $Destination 'bicraft.exe'
try {
    Copy-Item -LiteralPath $source -Destination $target -Force
} catch {
    Write-Error "cannot write $target (is a bicraft MCP server still running?): $_"
    exit 1
}
$version = & $target --version
Write-Host "Installed $version"
Write-Host "  -> $target"

# the desktop app, when the archive has it: copied next to bicraft.exe, never registered here
$desktop = Join-Path $PSScriptRoot 'bicraft-desktop.exe'
if (Test-Path -LiteralPath $desktop -PathType Leaf) {
    $desktopTarget = Join-Path $Destination 'bicraft-desktop.exe'
    try {
        Copy-Item -LiteralPath $desktop -Destination $desktopTarget -Force
        Write-Host "  -> $desktopTarget (Start menu entry and Open with: bicraft-desktop.exe --register)"
    } catch {
        Write-Warning "cannot write $desktopTarget (is the desktop app running?): $_"
    }
}

$dir = (Resolve-Path -LiteralPath $Destination).Path.TrimEnd('\')
$onPath = $env:PATH -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ieq $dir }
if (-not $onPath) {
    Write-Host ''
    Write-Host "$dir is not on PATH. To add it for your user (new terminals pick it up):"
    Write-Host "  [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';$dir', 'User')"
}
Write-Host ''
Write-Host 'MCP clients: run `bicraft setup --dry-run` to see the registration plan, then'
Write-Host '`bicraft setup --client claude_code --yes` (or another client); see docs\mcp.md.'
