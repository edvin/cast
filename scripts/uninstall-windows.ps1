# Cast Server — Windows Uninstall Script

$taskName = "CastServer"

# Stop if running
Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue

# Remove the scheduled task
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

Write-Host "Cast Server uninstalled." -ForegroundColor Green
Write-Host "Note: Log files in your media directory\logs\ were not removed."
