<#
.SYNOPSIS
    Keeps every core busy until Enter is pressed. For measurement runs in the
    test VM only (.ai/MEASUREMENTS.md, run b).

.DESCRIPTION
    Starts one busy loop per logical processor at normal priority, so streaming
    competes with a fully loaded machine the way a heavy app would make it.
    Closing the window or pressing Enter stops them all.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$cores = [Environment]::ProcessorCount
$jobs = 1..$cores | ForEach-Object {
    Start-Job -ScriptBlock { while ($true) { $null = [Math]::Sqrt(12345.678) } }
}
Write-Host "$cores busy loops running. Press Enter to stop them."
try {
    [void][Console]::ReadLine()
} finally {
    $jobs | Stop-Job -PassThru | Remove-Job
    Write-Host 'Stopped.'
}
