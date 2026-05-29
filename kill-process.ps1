# Kill all AI Desktop Pet processes
$processes = Get-Process | Where-Object {
    $_.ProcessName -like '*ai-desktop-pet*' -or
    $_.MainWindowTitle -like '*AI Desktop Pet*' -or
    $_.Path -like '*ai-desktop-pet*'
}

if ($processes.Count -gt 0) {
    foreach ($p in $processes) {
        Write-Host "Stopping: $($p.ProcessName) (PID: $($p.Id))"
        Stop-Process -Id $p.Id -Force
    }
    Write-Host "All processes stopped."
    Start-Sleep -Seconds 2
} else {
    Write-Host "No running AI Desktop Pet processes found."
}
