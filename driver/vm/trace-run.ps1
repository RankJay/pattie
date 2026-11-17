<#
.SYNOPSIS
    Records what the driver does during one measurement run. Runs INSIDE THE TEST
    VM, as Administrator.

.DESCRIPTION
    Starts recording, waits while you do the run in another window, then stops on
    Enter and leaves the files next to this script:

      <Name>-driver.etl  the driver's own events ("Pattie.Driver", src/trace.h),
                         recorded with Windows' logman
      <Name>-cpu.etl     with -Cpu: WPR's built-in CPU and Audio profiles (CPU
                         sampling, DPC/ISR, audio-engine glitches). Keep these
                         runs to about 60 s; the file grows fast.

    Copy them to the host and read each with analyze-trace.ps1.

    A custom WPR profile for the driver's provider was refused by wpr ("An Event
    session cannot be started without any providers", 0xc5580612; 2026-11-07),
    hence logman for the driver's events and WPR's own profiles for the rest.

.EXAMPLE
    .\trace-run.ps1 a-mic -Cpu
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Name,
    [switch]$Cpu
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$session = 'PattieDriver'
. (Join-Path $here 'trace-provider.ps1')
$provider = $TraceProviderGuid
$driverEtl = Join-Path $here "$Name-driver.etl"
$cpuEtl = Join-Path $here "$Name-cpu.etl"

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run this as Administrator.' }

# A session left over from an interrupted run would block the new one.
& logman stop $session -ets *> $null

# All keywords, level 5 (verbose): every Pattie event.
& logman start $session -p $provider 0xffffffffffffffff 5 -o $driverEtl -ets
if ($LASTEXITCODE -ne 0) { throw "logman start failed ($LASTEXITCODE)" }

$cpuStarted = $false
try {
    if ($Cpu) {
        & wpr -start CPU -start Audio -filemode
        if ($LASTEXITCODE -ne 0) { throw "wpr -start failed ($LASTEXITCODE)" }
        $cpuStarted = $true
    }
    Write-Host ''
    Write-Host "Recording '$Name'. Do the run in another window, then press Enter here."
    [void][Console]::ReadLine()
} finally {
    if ($cpuStarted) {
        & wpr -stop $cpuEtl
        if ($LASTEXITCODE -ne 0) { Write-Warning "wpr -stop failed ($LASTEXITCODE)" }
    }
    & logman stop $session -ets
    if ($LASTEXITCODE -ne 0) { Write-Warning "logman stop failed ($LASTEXITCODE)" }
}

Write-Host ''
Get-Item $driverEtl, $cpuEtl -ErrorAction SilentlyContinue | Format-Table Name, Length -AutoSize | Out-Host
Write-Host 'Copy these to the host and run analyze-trace.ps1 on each.'
