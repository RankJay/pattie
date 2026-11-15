<#
.SYNOPSIS
    Installs the Viaduct driver. Runs INSIDE THE TEST VM ONLY.

.DESCRIPTION
    Refuses to run outside a VMware virtual machine. Two modes:

    Test-signed (default): needs test signing on (prepare-vm.ps1). Trusts the
    package's test certificate (in this VM only) and installs viaduct.inf from
    this folder.

    -Signed <folder>, the gate run: needs Secure Boot on and test signing off
    (the "clean" snapshot). Trusts nothing; checks that the folder's catalog is
    signed by Microsoft (attestation signing) and installs its viaduct.inf.

    Then, in both: installs with `viaduct-cli install` (creates the root device
    Root\Viaduct if there is none, installs the package on it), and with
    -Verifier turns on Driver Verifier for viaduct.sys (reboot needed).

    To undo, revert the VM to the snapshot it was installed on.
#>
[CmdletBinding()]
param(
    # The folder holding Microsoft's signed package (viaduct.inf, .sys, .cat).
    [string]$Signed,
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

$testSigning = ((& bcdedit /enum '{current}') -join "`n") -match '(?mi)^testsigning\s+Yes'

if ($Signed) {
    if ($testSigning) { throw 'Test signing is on: the gate run needs the "clean" snapshot (see .ai/ops/vm-steps.md, section H).' }
    if (-not (Confirm-SecureBootUEFI)) { throw 'Secure Boot is off: the gate run needs it on (the "clean" snapshot).' }
    $package = (Resolve-Path $Signed).Path
    $catalog = Get-AuthenticodeSignature (Join-Path $package 'viaduct.cat')
    if ($catalog.Status -ne 'Valid' -or $catalog.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw "viaduct.cat is not Microsoft-signed ($($catalog.Status), $($catalog.SignerCertificate.Subject))."
    }
    Write-Host "== Microsoft-signed package: $($catalog.SignerCertificate.Subject)"
} else {
    if (-not $testSigning) {
        throw 'Test signing is off. Run prepare-vm.ps1 first and reboot (see .ai/ops/vm-steps.md).'
    }
    $package = $here
    Write-Host '== Trusting the test certificate (this VM only)'
    $cer = Join-Path $here 'viaduct-test.cer'
    # certutil rather than Import-Certificate: the latter is refused with
    # E_ACCESSDENIED for the machine Root store on recent Windows 11, even elevated.
    foreach ($store in 'Root', 'TrustedPublisher') {
        & certutil -f -addstore $store $cer
        if ($LASTEXITCODE -ne 0) { throw "certutil -addstore $store failed ($LASTEXITCODE)" }
    }
}

Write-Host '== Installing the driver (viaduct-cli install, ADR 0016)'
& (Join-Path $here 'viaduct-cli.exe') install $package
$installExit = $LASTEXITCODE
# 3010: installed, finished at the next restart (an app holds a Device; ADR 0024).
if ($installExit -notin 0, 3010) { throw "viaduct-cli install failed ($installExit)" }
if ($installExit -eq 3010) { Write-Host '== Exit 3010: Windows finishes the update at the next restart' }

if ($Verifier) {
    Write-Host '== Turning on Driver Verifier for viaduct.sys'
    & verifier /standard /driver viaduct.sys
    Write-Host 'Reboot the VM for Driver Verifier to take effect.'
}

Write-Host ''
Write-Host 'Installed. Next: .\viaduct-cli.exe mic "Agent Mic"   (checks in platforms/windows/driver/README.md)'
