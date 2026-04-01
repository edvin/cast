# Cast Server - Windows Install Script
# Run from PowerShell
#
# Usage (with .env file - recommended):
#   Place .env next to cast-server.exe, then run:
#   .\install-windows.ps1
#
# Usage (with parameters):
#   .\install-windows.ps1 -MediaPath "D:\Shows" -TmdbKey "your-key"

param(
    [string]$MediaPath = "",
    [string]$TmdbKey = "",
    [int]$Port = 3456,
    [string]$ServerName = ""
)

$ErrorActionPreference = "Stop"

# Find the cast-server binary
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$serverExe = Join-Path $scriptDir "cast-server.exe"

if (-not (Test-Path $serverExe)) {
    $serverExe = Join-Path (Split-Path -Parent $scriptDir) "server\target\release\cast-server.exe"
}

if (-not (Test-Path $serverExe)) {
    Write-Error "cast-server.exe not found. Place it next to this script or build with: cargo build --release"
    exit 1
}

$serverDir = Split-Path -Parent $serverExe
$envFile = Join-Path $serverDir ".env"

# If no MediaPath parameter, try reading from .env
if (-not $MediaPath) {
    if (Test-Path $envFile) {
        Get-Content $envFile | ForEach-Object {
            if ($_ -match '^\s*([^#][^=]+)=(.*)$') {
                $key = $Matches[1].Trim()
                $val = $Matches[2].Trim().Trim('"').Trim("'")
                if ($key -eq "CAST_MEDIA_PATH") { $MediaPath = $val }
                if ($key -eq "TMDB_API_KEY" -and -not $TmdbKey) { $TmdbKey = $val }
                if ($key -eq "CAST_SERVER_NAME" -and -not $ServerName) { $ServerName = $val }
            }
        }
        Write-Host "Read configuration from $envFile" -ForegroundColor Cyan
    }
}

if (-not $MediaPath) {
    Write-Error "Media path is required. Pass -MediaPath or set CAST_MEDIA_PATH in .env next to the binary."
    exit 1
}

if (-not (Test-Path $MediaPath)) {
    Write-Error "Media directory does not exist: $MediaPath"
    exit 1
}

# Build arguments
$arguments = "--media ""$MediaPath"" --port $Port --log-file"
if ($ServerName) { $arguments += " --name ""$ServerName""" }
if ($TmdbKey) { $arguments += " --tmdb-key $TmdbKey" }

# Create scheduled task
$taskName = "CastServer"
$action = New-ScheduledTaskAction -Execute $serverExe -Argument $arguments -WorkingDirectory $serverDir
$trigger = New-ScheduledTaskTrigger -AtLogOn

$settingsParams = @{
    AllowStartIfOnBatteries = $true
    DontStopIfGoingOnBatteries = $true
    StartWhenAvailable = $true
    ExecutionTimeLimit = [TimeSpan]::Zero
    RestartCount = 3
    RestartInterval = [TimeSpan]::FromMinutes(1)
}
$settings = New-ScheduledTaskSettingsSet @settingsParams

# Remove existing task if present
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Description "Cast media server"

Write-Host ""
Write-Host "Cast Server installed!" -ForegroundColor Green
Write-Host ""
Write-Host "  Task Name:   $taskName"
Write-Host "  Executable:  $serverExe"
Write-Host "  Media Path:  $MediaPath"
Write-Host "  Port:        $Port"
if ($ServerName) { Write-Host "  Name:        $ServerName" }
Write-Host "  Logs:        $MediaPath\logs"
Write-Host ""
Write-Host "Starting server now..."

Start-ScheduledTask -TaskName $taskName

Write-Host "Server is running. It will auto-start on login." -ForegroundColor Green
Write-Host ""
Write-Host "  Stop:    Stop-ScheduledTask -TaskName CastServer"
Write-Host "  Remove:  Unregister-ScheduledTask -TaskName CastServer"
