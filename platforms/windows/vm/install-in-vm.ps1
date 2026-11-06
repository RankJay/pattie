<#
.SYNOPSIS
    Installs the test-signed Viaduct driver. Runs INSIDE THE TEST VM ONLY.

.DESCRIPTION
    Refuses to run unless test signing is on, which it never is on the host
    (.ai/VM-PLAN.md). Then:
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

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run this as Administrator.' }

$bootConfig = (& bcdedit /enum '{current}') -join "`n"
if ($bootConfig -notmatch '(?mi)^testsigning\s+Yes') {
    throw 'Test signing is off. This script only runs inside the prepared test VM (see .ai/VM-PLAN.md).'
}

Write-Host '== Trusting the test certificate (this VM only)'
$cer = Join-Path $here 'viaduct-test.cer'
Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null

Write-Host '== Installing the driver package'
& pnputil /add-driver (Join-Path $here 'viaduct.inf') /install
if ($LASTEXITCODE -ne 0) { throw "pnputil failed ($LASTEXITCODE)" }

Write-Host '== Creating the root device'
& (Join-Path $here 'devgen.exe') /add /hardwareid 'Root\Viaduct'
if ($LASTEXITCODE -ne 0) { throw "devgen failed ($LASTEXITCODE)" }

if ($Verifier) {
    Write-Host '== Turning on Driver Verifier for viaduct.sys'
    & verifier /standard /driver viaduct.sys
    Write-Host 'Reboot the VM for Driver Verifier to take effect.'
}

Write-Host ''
Write-Host 'Installed. Next: .\viaduct-cli.exe mic "Agent Mic"   (checks in platforms/windows/driver/README.md)'
