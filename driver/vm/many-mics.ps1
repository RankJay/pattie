<#
.SYNOPSIS
    Runs several `pattie-cli mic` instances at once from one window. For the
    test VM's run (c), 8 Devices at once (.ai/status/measurements.md).

.DESCRIPTION
    Starts -Count separate pattie-cli processes (separate Owners, as separate
    apps would be), each creating a Microphone named "<Prefix> 1", "<Prefix> 2",
    ... and streaming its tone. Each one's output goes to a log next to this
    script. Press Enter here to stop them all cleanly (each gets its own Enter),
    then the last report line of each is shown: Gap and Overflow figures included.

    Each CLI stops on Enter or at end of input, so each gets a stdin pipe that
    stays open until it is told to stop.

.EXAMPLE
    .\many-mics.ps1            # 7 mics, "Mic 1" .. "Mic 7"
#>
[CmdletBinding()]
param(
    [ValidateRange(1, 8)][int]$Count = 7,
    [string]$Prefix = 'Mic'
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$cli = Join-Path $here 'pattie-cli.exe'
if (-not (Test-Path $cli)) { throw "Missing $cli" }

$mics = @()
for ($i = 1; $i -le $Count; $i++) {
    $name = "$Prefix $i"
    $log = Join-Path $here ("many-mics-{0}.log" -f $i)
    # cmd.exe redirects the CLI's output to the log and hands it our stdin pipe.
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = 'cmd.exe'
    $info.Arguments = '/c ""{0}" mic "{1}" > "{2}" 2>&1"' -f $cli, $name, $log
    $info.UseShellExecute = $false
    $info.RedirectStandardInput = $true
    $info.CreateNoWindow = $true
    $mics += [pscustomobject]@{ Name = $name; Log = $log; Process = [System.Diagnostics.Process]::Start($info) }
}

Start-Sleep -Seconds 3
$running = @($mics | Where-Object { -not $_.Process.HasExited }).Count
Write-Host "$running of $Count microphones running ($Prefix 1 .. $Prefix $Count). Logs: many-mics-N.log"
if ($running -lt $Count) {
    Write-Warning 'Some stopped at once; their logs below say why.'
}
Write-Host 'Press Enter to stop them all.'
[void][Console]::ReadLine()

foreach ($mic in $mics) {
    if (-not $mic.Process.HasExited) {
        try { $mic.Process.StandardInput.WriteLine() } catch { }
    }
}
foreach ($mic in $mics) {
    if (-not $mic.Process.WaitForExit(5000)) {
        Write-Warning "$($mic.Name) didn't stop within 5 s; ending it."
        try { $mic.Process.Kill() } catch { }
    }
}

Write-Host ''
foreach ($mic in $mics) {
    $last = Get-Content $mic.Log -ErrorAction SilentlyContinue |
        Where-Object { $_ -match 'gap|error|couldn|Stopped' } | Select-Object -Last 2
    Write-Host ("{0,-8} {1}" -f $mic.Name, ($last -join '  |  '))
}
