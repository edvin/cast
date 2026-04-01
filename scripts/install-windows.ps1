# Cast Server — Windows Install Script
# Run as Administrator (or current user for per-user install)
#
# Usage:
#   .\install-windows.ps1 -MediaPath "D:\Shows" -TmdbKey "your-key"
#   .\install-windows.ps1 -MediaPath "D:\Shows"   # without TMDB

param(
    [Parameter(Mandatory=$true)]
    [string]$MediaPath,

    [string]$TmdbKey = "",

    [int]$Port = 3456,

    [string]$ServerName = "Cast Server"
)

$ErrorActionPreference = "Stop"

# Find the cast-server binary
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$serverExe = Join-Path (Split-Path -Parent $scriptDir) "server\target\release\cast-server.exe"

if (-not (Test-Path $serverExe)) {
    # Try next to the script
    $serverExe = Join-Path $scriptDir "cast-server.exe"
}

if (-not (Test-Path $serverExe)) {
    Write-Error "cast-server.exe not found. Build with: cargo build --release --manifest-path server/Cargo.toml"
    exit 1
}

if (-not (Test-Path $MediaPath)) {
    Write-Error "Media directory does not exist: $MediaPath"
    exit 1
}

# Build arguments
$arguments = "--media `"$MediaPath`" --port $Port --name `"$ServerName`" --log-file"
if ($TmdbKey) {
    $arguments += " --tmdb-key $TmdbKey"
}

# Create scheduled task to run at logon
$taskName = "CastServer"
$action = New-ScheduledTaskAction -Execute $serverExe -Argument $arguments
$trigger = New-ScheduledTaskTrigger -AtLogOn
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 3 `
    -RestartInterval ([TimeSpan]::FromMinutes(1))

# Remove existing task if present
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

Register-ScheduledTask `
    -TaskName $taskName `
    -Action $action `
    -Trigger $trigger `
    -Settings $settings `
    -Description "Cast media server — streams video to Apple TV"

Write-Host ""
Write-Host "Cast Server installed successfully!" -ForegroundColor Green
Write-Host ""
Write-Host "  Task Name:      $taskName"
Write-Host "  Executable:     $serverExe"
Write-Host "  Media Path:     $MediaPath"
Write-Host "  Port:           $Port"
Write-Host "  Log files:      $MediaPath\logs\"
Write-Host ""
Write-Host "Starting server now..."

# Start it immediately
Start-ScheduledTask -TaskName $taskName

Write-Host "Server is running. It will auto-start on login."
Write-Host "Logs: $MediaPath\logs\cast-server.log.*"
Write-Host ""
Write-Host "To stop:    Stop-ScheduledTask -TaskName CastServer"
Write-Host "To remove:  .\uninstall-windows.ps1"
