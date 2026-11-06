<#
.SYNOPSIS
    Installs the test-signed Viaduct driver. Runs INSIDE THE TEST VM ONLY.

.DESCRIPTION
    Refuses to run unless this is a VMware virtual machine with test signing on
    (prepare-vm.ps1 sets that up; the host has neither). Then:
      1. trusts the package's test certificate (in this VM only),
      2. adds and installs the driver package with pnputil,
      3. creates the root device (Root\Viaduct) with devgen,
      4. with -Verifier, turns on Driver Verifier for viaduct.sys (reboot needed).

    To undo, revert the VM to its "debug-ready" snapshot.
#>
[CmdletBinding()]
param(
    [switch]$Verifier
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot

$system = Get-CimInstance Win32_ComputerSystem
if ($system.Manufacturer -notmatch 'VMware' -and $system.Model -notmatch 'VMware') {
    throw "This is not a VMware virtual machine ($($system.Manufacturer) $($system.Model)). Refusing to install."
}

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run this as Administrator.' }

$bootConfig = (& bcdedit /enum '{current}') -join "`n"
if ($bootConfig -notmatch '(?mi)^testsigning\s+Yes') {
    throw 'Test signing is off. Run prepare-vm.ps1 first and reboot (see .ai/VM-STEPS.md).'
}

Write-Host '== Trusting the test certificate (this VM only)'
$cer = Join-Path $here 'viaduct-test.cer'
# certutil rather than Import-Certificate: the latter is refused with
# E_ACCESSDENIED for the machine Root store on recent Windows 11, even elevated.
foreach ($store in 'Root', 'TrustedPublisher') {
    & certutil -f -addstore $store $cer
    if ($LASTEXITCODE -ne 0) { throw "certutil -addstore $store failed ($LASTEXITCODE)" }
}

Write-Host '== Installing the driver package'
& pnputil /add-driver (Join-Path $here 'viaduct.inf') /install
if ($LASTEXITCODE -ne 0) { throw "pnputil failed ($LASTEXITCODE)" }

Write-Host '== Creating the root device'
# /bus ROOT: devgen's default (SWD) device disappears at the next reboot.
& (Join-Path $here 'devgen.exe') /add /bus ROOT /hardwareid 'Root\Viaduct'
if ($LASTEXITCODE -ne 0) { throw "devgen failed ($LASTEXITCODE)" }

if ($Verifier) {
    Write-Host '== Turning on Driver Verifier for viaduct.sys'
    & verifier /standard /driver viaduct.sys
    Write-Host 'Reboot the VM for Driver Verifier to take effect.'
}

Write-Host ''
Write-Host 'Installed. Next: .\viaduct-cli.exe mic "Agent Mic"   (checks in platforms/windows/driver/README.md)'
