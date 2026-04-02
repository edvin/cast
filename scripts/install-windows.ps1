# Cast Server - Windows Install Script
# Creates a scheduled task that auto-starts the server at login.
# Configure the server via a .env file next to the binary.
#
# Usage:
#   1. Place cast-server*.exe and .env in a folder
#   2. Run: .\install-windows.ps1

$ErrorActionPreference = "Stop"

# Find the cast-server binary
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$serverExe = Get-ChildItem -Path $scriptDir -Filter "cast-server*.exe" -File | Select-Object -First 1 -ExpandProperty FullName

if (-not $serverExe) {
    Write-Error "No cast-server*.exe found in $scriptDir. Place the binary next to this script."
    exit 1
}

$serverDir = Split-Path -Parent $serverExe

# Create scheduled task
$taskName = "CastServer"
$action = New-ScheduledTaskAction -Execute $serverExe -Argument "--log-file" -WorkingDirectory $serverDir
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

$principal = New-ScheduledTaskPrincipal -UserId "$env:USERNAME" -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Description "Cast media server"

# Hide the console window
$task = Get-ScheduledTask -TaskName $taskName
$task.Settings.Hidden = $true
$task | Set-ScheduledTask | Out-Null

Write-Host ""
Write-Host "Cast Server installed!" -ForegroundColor Green
Write-Host ""
Write-Host "  Binary:  $serverExe"
Write-Host "  Config:  $serverDir\.env"
Write-Host ""
Write-Host "Starting server now..."

Start-ScheduledTask -TaskName $taskName

Write-Host "Server is running. It will auto-start on login." -ForegroundColor Green
Write-Host ""
Write-Host "  Stop:    Stop-ScheduledTask -TaskName CastServer"
Write-Host "  Remove:  Unregister-ScheduledTask -TaskName CastServer"
