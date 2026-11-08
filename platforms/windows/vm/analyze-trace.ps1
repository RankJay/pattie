<#
.SYNOPSIS
    Reads a trace recorded in the test VM with trace-run.ps1. Runs on the host.

.DESCRIPTION
    Prints, using the Windows Performance Toolkit's xperf.exe (installed with the
    Driver Kit):
      1. the driver's own events ("Viaduct.Driver"): stream format, packet size,
         and per stream the timer's late passes, stalls skipped and lateness plus the Ring's Gap
         and Overflow totals (src/trace.h);
      2. for a -cpu.etl recording, the DPC/ISR summary, with the lines for
         ntoskrnl.exe, where the stream timer's DPCs are counted.

    Read-only: it only reads the trace. Light: xperf is stopped above -MaxMB, as
    the host also runs the VM (see analyze-minidump.ps1).

.PARAMETER Path
    The .etl file copied from the guest.

.EXAMPLE
    .\analyze-trace.ps1 D:\VMs\traces\mic-idle.etl
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [int]$MaxMB = 1024
)

$ErrorActionPreference = 'Stop'
$xperf = 'C:\Program Files (x86)\Windows Kits\10\Windows Performance Toolkit\xperf.exe'
$provider = '{a97790c8-3349-54e2-c7f3-9ad24f60a4ce}' # "Viaduct.Driver" (src/trace.c)

if (-not (Test-Path $xperf)) { throw "Missing xperf.exe: $xperf (installed with the Driver Kit)" }
if (-not (Test-Path $Path)) { throw "No such trace: $Path" }

# Runs xperf under a memory and time watchdog; returns its output lines.
function Invoke-Xperf([string[]]$Arguments) {
    $out = [IO.Path]::GetTempFileName()
    $err = [IO.Path]::GetTempFileName()
    try {
        $process = Start-Process -FilePath $xperf -NoNewWindow -PassThru `
            -RedirectStandardOutput $out -RedirectStandardError $err -ArgumentList $Arguments
        $clock = [Diagnostics.Stopwatch]::StartNew()
        while (-not $process.HasExited) {
            $process.Refresh()
            if ($process.WorkingSet64 -gt $MaxMB * 1MB -or $clock.Elapsed.TotalSeconds -gt 300) {
                Stop-Process -Id $process.Id -Force
                throw "xperf stopped: it passed $MaxMB MB or 5 minutes."
            }
            Start-Sleep -Milliseconds 300
        }
        return Get-Content $out
    } finally {
        Remove-Item $out, $err -ErrorAction SilentlyContinue
    }
}

Write-Host '== Viaduct driver events'
$events = Invoke-Xperf @('-i', "`"$Path`"", '-a', 'dumper', '-provider', $provider, '-add_fieldnames')
$ours = $events | Where-Object { $_ -match 'Viaduct\.Driver|a97790c8' }
if (-not $ours) {
    Write-Host '   (none: was the driver loaded and a stream started while recording?)'
} else {
    $ours | ForEach-Object { '   ' + ($_ -replace '\s{2,}', ' ').Trim() }
}

$dpc = Invoke-Xperf @('-i', "`"$Path`"", '-a', 'dpcisr', '-summary')
if ($dpc -match 'DPC') {
    Write-Host '== DPC/ISR summary'
    $dpc | Where-Object { $_.Trim() } | Select-Object -First 60 | ForEach-Object { '   ' + $_ }
    # The stream timer is a WDF high-resolution timer, which runs on the kernel's
    # own timers: its DPCs are counted under ntoskrnl.exe, not viaduct.sys or
    # Wdf01000.sys (seen 2026-11-07). Estimate the driver's share against a
    # recording with no stream running: extra DPCs (about 100/s per stream, one
    # per 10 ms packet) and the rise in ntoskrnl's DPC time.
    Write-Host '== DPC lines for ntoskrnl.exe (the stream timer), viaduct.sys and Wdf01000.sys'
    $dpc | Select-String -Pattern 'ntoskrnl|viaduct|Wdf01000' | ForEach-Object { '   ' + $_.Line.Trim() }
    $dpc | Select-String -Pattern '^Total = \d+$' | Select-Object -First 1 | ForEach-Object { '   DPCs in the whole trace: ' + $_.Line.Trim() }
} else {
    Write-Host '== No DPC/ISR data (only -cpu.etl recordings have it).'
}
