<#
.SYNOPSIS
    Prepares the test VM for loading Viaduct. Runs INSIDE THE VMWARE GUEST ONLY.

.DESCRIPTION
    Refuses to run unless this is a VMware virtual machine with Secure Boot already
    turned off (in the VM's firmware settings). Then:
      1. turns test signing on (this VM only),
      2. makes a crash write a small memory dump (C:\Windows\Minidump, about 1 MB)
         instead of a full one, for analyze-minidump.ps1 on the host,
      3. turns kernel debugging off.
    Reboot the VM afterwards.

    No live kernel debugger: on a host where Windows' own hypervisor runs (VBS,
    WSL), VMware guests bugcheck with NMI_HARDWARE_FAILURE (0x80) whenever one is
    attached, a known, unfixed VMware issue (.ai/VM-FINDINGS.md). Crashes are analysed
    from their minidumps instead.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$system = Get-CimInstance Win32_ComputerSystem
if ($system.Manufacturer -notmatch 'VMware' -and $system.Model -notmatch 'VMware') {
    throw "This is not a VMware virtual machine ($($system.Manufacturer) $($system.Model)). Refusing to change boot settings."
}

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run this as Administrator.' }

$secureBoot = $false
try { $secureBoot = Confirm-SecureBootUEFI } catch { $secureBoot = $false }
if ($secureBoot) {
    throw 'Secure Boot is on. Power the VM off, turn Secure Boot off in VM Settings > Options > Advanced, then run this again.'
}

Write-Host '== Turning test signing on (this VM only)'
& bcdedit /set testsigning on
if ($LASTEXITCODE -ne 0) { throw "bcdedit testsigning failed ($LASTEXITCODE)" }

Write-Host '== Turning kernel debugging off'
& bcdedit /debug off
if ($LASTEXITCODE -ne 0) { throw "bcdedit debug failed ($LASTEXITCODE)" }

Write-Host '== Making crashes write small memory dumps'
# CrashDumpEnabled 3 = small memory dump, written to MinidumpDir (%SystemRoot%\Minidump).
Set-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl' -Name CrashDumpEnabled -Value 3 -Type DWord

Write-Host ''
Write-Host 'Done. Reboot the VM now. After it restarts you should see "Test Mode" in the bottom-right corner.'
