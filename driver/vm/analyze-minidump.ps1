<#
.SYNOPSIS
    Reads a guest crash minidump on the host, cheaply. Runs on the host.

.DESCRIPTION
    Prints the bug check, which pattie.sys build was loaded, and the driver frames
    (with source lines) on the crashing thread's stack, using only the WDK's kd and
    the driver's own symbols from target\driver\x64\<Configuration>.

    Deliberately light: no !analyze -v and no Microsoft symbol server. Those load
    hundreds of MB of symbols and extensions, and with the VM running they filled
    this PC's memory (2026-11-07). kd is also stopped if it ever passes -MaxMB.
    Use a small memory dump (C:\Windows\Minidump\*.dmp, ~1 MB; prepare-vm.ps1
    sets the guest to write those), never the full MEMORY.DMP.

    Read-only: it only reads the dump file.

.PARAMETER Path
    The minidump copied from the guest's C:\Windows\Minidump.

.EXAMPLE
    .\analyze-minidump.ps1 D:\VMs\dumps\110726-12345-01.dmp
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [string]$Configuration = 'Release',
    [int]$MaxMB = 1024
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$kd = 'C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\kd.exe'
$symbols = Join-Path $repo "target\driver\x64\$Configuration"

if (-not (Test-Path $kd)) { throw "Missing kd.exe: $kd (installed with the WDK)" }
if (-not (Test-Path $Path)) { throw "No such dump: $Path" }
if ((Get-Item $Path).Length -gt 64MB) {
    throw "That looks like a full memory dump. Use a minidump from the guest's C:\Windows\Minidump instead."
}

# Runs kd with a command list, under a memory and time watchdog; returns its output lines.
function Invoke-Kd([string]$Commands) {
    $out = [IO.Path]::GetTempFileName()
    $err = [IO.Path]::GetTempFileName()
    $saved = $env:_NT_SYMBOL_PATH, $env:_NT_ALT_SYMBOL_PATH
    $env:_NT_SYMBOL_PATH = ''      # never reach the Microsoft symbol server
    $env:_NT_ALT_SYMBOL_PATH = ''
    try {
        $process = Start-Process -FilePath $kd -NoNewWindow -PassThru `
            -RedirectStandardOutput $out -RedirectStandardError $err `
            -ArgumentList @('-z', "`"$Path`"", '-y', "`"$symbols`"", '-i', "`"$symbols`"", '-c', "`"$Commands; q`"")
        $clock = [Diagnostics.Stopwatch]::StartNew()
        while (-not $process.HasExited) {
            $process.Refresh()
            if ($process.WorkingSet64 -gt $MaxMB * 1MB -or $clock.Elapsed.TotalSeconds -gt 120) {
                Stop-Process -Id $process.Id -Force
                throw "kd stopped: it passed $MaxMB MB or 2 minutes."
            }
            Start-Sleep -Milliseconds 300
        }
        return Get-Content $out
    } finally {
        $env:_NT_SYMBOL_PATH, $env:_NT_ALT_SYMBOL_PATH = $saved
        Remove-Item $out, $err -ErrorAction SilentlyContinue
    }
}

$first = Invoke-Kd '.bugcheck; .echo ===LM; lmvm pattie'
$bugcheck = $first | Select-String -Pattern '^Bugcheck code\s+([0-9A-Fa-f]+)' | Select-Object -First 1
$arguments = $first | Select-String -Pattern '^Arguments\s+(.+)$' | Select-Object -First 1
if (-not $bugcheck) { $first; throw 'kd did not report a bug check; is this a kernel minidump?' }

$code = [Convert]::ToUInt32($bugcheck.Matches[0].Groups[1].Value, 16) -band 0x0FFFFFFF
$args4 = @($arguments.Matches[0].Groups[1].Value -split '\s+' | ForEach-Object { $_ -replace '`', '' })

Write-Host ("== Bug check 0x{0:X} ({1})" -f $code, ($args4 -join ', '))
$first | Select-String -Pattern 'Image path|Timestamp|Loaded symbol' | ForEach-Object { '   ' + $_.Line.Trim() }

# Exceptions carry the faulting thread's context: switch to it before walking the stack.
$context = switch ($code) {
    0x7E { ".cxr 0x$($args4[3])" }   # SYSTEM_THREAD_EXCEPTION_NOT_HANDLED: context record
    0x8E { ".trap 0x$($args4[2])" }  # KERNEL_MODE_EXCEPTION_NOT_HANDLED: trap frame
    0x3B { ".cxr 0x$($args4[2])" }   # SYSTEM_SERVICE_EXCEPTION: context record
    default { '' }
}
$second = Invoke-Kd "$context; .lines -e; .reload /f pattie.sys; .echo ===REGS; r; .echo ===STACK; dps @rsp L200"

Write-Host '== Faulting instruction'
$second | Select-String -Pattern '^[A-Za-z0-9_]+(!\w+)?\+0x[0-9a-f]+:$|^[A-Za-z0-9_]+!\S+:$|^fffff[0-9a-f`]+\s+[0-9a-f]+\s+\w+' |
    Select-Object -First 2 | ForEach-Object { '   ' + $_.Line.Trim() }

Write-Host '== Driver frames on the stack (newest first; values below the crash frame can be stale)'
$second | Select-String -Pattern '^[0-9a-f`]+\s+[0-9a-f`]+\s+(pattie|Acx01000|Wdf01000)' |
    Select-Object -First 25 | ForEach-Object { '   ' + $_.Line.Trim() }
