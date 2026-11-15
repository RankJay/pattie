<#
.SYNOPSIS
    Builds the installer (ADR 0024): one per-machine MSI that puts the driver
    and viaduct-cli on a machine. Runs on the host; it installs nothing.

.DESCRIPTION
    1. Reads the payload folder (viaduct.inf, viaduct.sys, viaduct.cat): the MSI
       carries it byte for byte. The product version X.Y.Z is its DriverVer's
       first three numbers (DriverVer is X.Y.Z.B; Windows Installer compares only
       three, so each MSI needs its own X.Y.Z).
    2. -Release refuses, listing every reason:
       - a catalog that isn't signed by Microsoft (Partner Center's output is),
         or that doesn't cover the payload's own viaduct.inf and viaduct.sys;
       - a viaduct.sys that isn't signed by Microsoft;
       - no release UpgradeCode in identity.wxi: it is created only when the
         product is named, and must never change after;
       - a viaduct-cli.exe (-Cli) without a valid Authenticode signature.
       Without -Release it builds a test MSI (identity.wxi's test UpgradeCode,
       "-test" in its name) from any payload, e.g. a VM test package.
    3. Builds the custom actions (crates/viaduct-msi) and, for a test MSI,
       viaduct-cli, with the C runtime linked in (+crt-static): a fresh machine
       may lack VCRUNTIME140.dll. Into target\msi\build.
    4. Runs WiX v6 (`wix build`) on viaduct.wxs into target\msi.

    The release MSI is then Authenticode-signed by hand with the EV certificate
    (the script prints the command), as the attestation cab is.

    Needs the .NET SDK and WiX v6 (`dotnet tool install --global wix`):
    .ai/ops/vm-host-changes.md, change 24. VM checks: .ai/ops/vm-steps.md, R3.
#>
[CmdletBinding()]
param(
    # The driver package folder: a VM test package (target\vm-package...) for a
    # test MSI, Partner Center's signed output for a release.
    [Parameter(Mandatory)]
    [string]$Payload,

    # A release MSI: Microsoft-signed payload, the real UpgradeCode, a signed CLI.
    [switch]$Release,

    # The viaduct-cli.exe to ship. Required with -Release (built, then signed by
    # hand); a test MSI builds its own when it's left out.
    [string]$Cli
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$repo = (Resolve-Path (Join-Path $here '..\..\..')).Path
$out = Join-Path $repo 'target\msi'
$build = Join-Path $out 'build'

# --- 1. The payload and its version.
$Payload = (Resolve-Path $Payload).Path
foreach ($file in 'viaduct.inf', 'viaduct.sys', 'viaduct.cat') {
    if (-not (Test-Path (Join-Path $Payload $file))) { throw "The payload has no $file ($Payload)" }
}
$stamped = Select-String -Path (Join-Path $Payload 'viaduct.inf') -Pattern '^\s*DriverVer\s*=\s*[^,]+,\s*(\d+)\.(\d+)\.(\d+)\.(\d+)\s*$'
if ($null -eq $stamped) { throw "No DriverVer with four numbers in $Payload\viaduct.inf" }
$g = $stamped.Matches[0].Groups
$productVersion = "$($g[1].Value).$($g[2].Value).$($g[3].Value)"
Write-Host "== Payload $Payload, DriverVer $($g[1].Value).$($g[2].Value).$($g[3].Value).$($g[4].Value), product $productVersion"

# --- 2. Identity, and what a release needs.
$identity = Get-Content -Raw (Join-Path $here 'identity.wxi')
function Get-Define([string]$name) {
    $m = [regex]::Match($identity, "<\?define\s+$name\s*=\s*""([^""]*)""\s*\?>")
    if (-not $m.Success) { throw "identity.wxi defines no $name" }
    $m.Groups[1].Value
}
$guid = '^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$'
$testCode = Get-Define 'TestUpgradeCode'
$releaseCode = Get-Define 'ReleaseUpgradeCode'

if ($Release) {
    $refusals = @()
    $catalog = Get-AuthenticodeSignature (Join-Path $Payload 'viaduct.cat')
    $signer = if ($catalog.SignerCertificate) { $catalog.SignerCertificate.Subject } else { '(unsigned)' }
    if ($catalog.Status -ne 'Valid' -or $signer -notmatch 'CN=Microsoft Windows Hardware Compatibility Publisher') {
        $refusals += "the catalog isn't signed by Microsoft (status $($catalog.Status), signer $signer): use Partner Center's output, byte for byte"
    } else {
        # Signed by Microsoft isn't enough: it must be *this* package's catalog,
        # covering its own INF and driver (any WHCP catalog passed the signer
        # check alone, 2026-11-15). signtool looks each file's hash up in it.
        $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
            Sort-Object FullName | Select-Object -Last 1
        if (-not $signtool) {
            $refusals += "signtool.exe not found (Windows SDK): can't check the catalog covers the payload"
        } else {
            foreach ($file in 'viaduct.inf', 'viaduct.sys') {
                # signtool writes to stderr on a mismatch: not an error for this
                # script, only its exit code counts.
                $saved = $ErrorActionPreference
                $ErrorActionPreference = 'Continue'
                & $signtool.FullName verify /q /pa /c (Join-Path $Payload 'viaduct.cat') (Join-Path $Payload $file) 2>&1 | Out-Null
                $covered = $LASTEXITCODE -eq 0
                $ErrorActionPreference = $saved
                if (-not $covered) {
                    $refusals += "the catalog doesn't cover $file (signtool verify /pa /c): it isn't this package's catalog"
                }
            }
        }
    }
    # Partner Center signs the driver binary itself too.
    $driver = Get-AuthenticodeSignature (Join-Path $Payload 'viaduct.sys')
    $driverSigner = if ($driver.SignerCertificate) { $driver.SignerCertificate.Subject } else { '(unsigned)' }
    if ($driver.Status -ne 'Valid' -or $driverSigner -notmatch 'CN=Microsoft Windows Hardware Compatibility Publisher') {
        $refusals += "viaduct.sys isn't signed by Microsoft (status $($driver.Status), signer $driverSigner)"
    }
    if ($releaseCode -notmatch $guid) {
        $refusals += "identity.wxi has no release UpgradeCode: it is created only once the product is named (ADR 0024)"
    } elseif ($releaseCode -eq $testCode) {
        $refusals += "the release UpgradeCode is the test one"
    }
    if (-not $Cli) {
        $refusals += "-Cli is required: the signed viaduct-cli.exe to ship"
    } elseif ((Get-AuthenticodeSignature $Cli).Status -ne 'Valid') {
        $refusals += "$Cli has no valid Authenticode signature: sign it first"
    }
    if ($refusals.Count -gt 0) {
        throw ("Refusing a release MSI:`n  - " + ($refusals -join "`n  - "))
    }
    $upgradeCode = $releaseCode
    $msiName = "viaduct-$productVersion.msi"
} else {
    if ($testCode -notmatch $guid) { throw "identity.wxi's TestUpgradeCode isn't a GUID" }
    $upgradeCode = $testCode
    $msiName = "viaduct-$productVersion-test.msi"
}

# --- 3. The custom actions (and a test CLI), C runtime linked in.
$wix = Get-Command wix -ErrorAction SilentlyContinue
if (-not $wix) { throw 'WiX v6 not found: dotnet tool install --global wix (vm-host-changes.md, change 24)' }
Write-Host '== Building the custom actions (crt-static)'
$packages = @('-p', 'viaduct-msi')
if (-not $Cli) { $packages += @('-p', 'viaduct-cli') }
$savedFlags = $env:RUSTFLAGS
$env:RUSTFLAGS = '-C target-feature=+crt-static'
try {
    & cargo build --release @packages --target-dir $build
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
} finally {
    $env:RUSTFLAGS = $savedFlags
}
$actions = Join-Path $build 'release\viaduct_msi.dll'
if (-not $Cli) { $Cli = Join-Path $build 'release\viaduct-cli.exe' }
$Cli = (Resolve-Path $Cli).Path

# --- 4. WiX.
New-Item -ItemType Directory -Force $out | Out-Null
$msi = Join-Path $out $msiName
Write-Host "== wix build -> $msi"
& wix build (Join-Path $here 'viaduct.wxs') -arch x64 -i $here `
    -d "ProductVersion=$productVersion" -d "UpgradeCode=$upgradeCode" `
    -d "Payload=$Payload" -d "Cli=$Cli" -d "ActionsDll=$actions" `
    -o $msi
if ($LASTEXITCODE -ne 0) { throw "wix build failed ($LASTEXITCODE)" }

Write-Host ''
Write-Host "Built $msi"
if ($Release) {
    Write-Host 'Sign it with the EV certificate before shipping, e.g.:'
    Write-Host "  signtool sign /fd SHA256 /tr <RFC 3161 timestamp URL> /td SHA256 <certificate options> `"$msi`""
} else {
    Write-Host 'A test MSI: unsigned, test UpgradeCode, for the VM only (vm-steps.md, R3).'
}
