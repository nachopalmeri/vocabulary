param(
    [string]$ProcessName = 'worddock',
    [int]$Seconds = 60
)

$ErrorActionPreference = 'Stop'
$process = Get-Process -Name $ProcessName | Select-Object -First 1
if (-not $process) { throw "Process '$ProcessName' is not running. Start WordDock first." }

$logicalProcessors = [Environment]::ProcessorCount
$start = Get-Date
$startCpu = $process.TotalProcessorTime.TotalSeconds
$initial = [pscustomobject]@{
    PrivateBytesMiB = [math]::Round($process.PrivateMemorySize64 / 1MB, 2)
    WorkingSetMiB   = [math]::Round($process.WorkingSet64 / 1MB, 2)
    Handles         = $process.HandleCount
}

Write-Host "WordDock idle measurement for $Seconds seconds. Do not interact with the card during this interval."
Write-Host "Initial: Private Bytes=$($initial.PrivateBytesMiB) MiB; Working Set=$($initial.WorkingSetMiB) MiB; Handles=$($initial.Handles)"
Start-Sleep -Seconds $Seconds

$process.Refresh()
$elapsed = ((Get-Date) - $start).TotalSeconds
$cpuSeconds = $process.TotalProcessorTime.TotalSeconds - $startCpu
$cpuPercent = if ($elapsed -gt 0 -and $logicalProcessors -gt 0) {
    100.0 * $cpuSeconds / $elapsed / $logicalProcessors
} else { 0.0 }

[pscustomobject]@{
    Process             = $process.ProcessName
    SampleSeconds       = [math]::Round($elapsed, 1)
    LogicalProcessors   = $logicalProcessors
    PrivateBytesMiB     = [math]::Round($process.PrivateMemorySize64 / 1MB, 2)
    WorkingSetMiB       = [math]::Round($process.WorkingSet64 / 1MB, 2)
    CpuPercentNormalized= [math]::Round($cpuPercent, 4)
    HandleCount         = $process.HandleCount
    PrivateBytesDeltaMiB= [math]::Round(($process.PrivateMemorySize64 / 1MB) - $initial.PrivateBytesMiB, 2)
    HandleDelta         = $process.HandleCount - $initial.Handles
} | Format-List
